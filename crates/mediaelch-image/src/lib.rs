//! Artwork download, resizing and on-disk layout.

use std::path::{Path, PathBuf};

use image::{imageops::FilterType, GenericImageView};
use mediaelch_core::{ImageKind, ImageRef, MediaItem};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("image download failed: {0}")]
    Download(#[from] reqwest::Error),
    #[error("downloaded image is empty")]
    Empty,
    #[error("cannot decode downloaded image: {0}")]
    Decode(#[from] image::ImageError),
    #[error("cannot write image to {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageConfig {
    pub max_poster_width: u32,
    pub max_backdrop_width: u32,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            max_poster_width: 1000,
            max_backdrop_width: 1920,
        }
    }
}

#[derive(Debug)]
pub struct DownloadResult {
    pub kind: ImageKind,
    pub local_path: Option<PathBuf>,
    pub error: Option<String>,
}

pub struct ImageDownloader {
    client: reqwest::Client,
    config: ImageConfig,
}

impl ImageDownloader {
    pub fn new(config: ImageConfig) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("MediaElch/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("valid HTTP client configuration");
        Self { client, config }
    }

    /// Download every image that has a source URL but no local file yet.
    pub async fn download_missing(&self, item: &MediaItem) -> Vec<DownloadResult> {
        let mut results = Vec::new();
        for image in &item.images {
            let Some(url) = image.source_url.as_deref().filter(|url| !url.is_empty()) else {
                continue;
            };
            if image.local_path.is_file() {
                continue;
            }
            let outcome = match self.fetch(image, url).await {
                Ok(path) => DownloadResult {
                    kind: image.kind,
                    local_path: Some(path),
                    error: None,
                },
                Err(error) => {
                    tracing::warn!(%error, %url, "image download failed");
                    DownloadResult {
                        kind: image.kind,
                        local_path: None,
                        error: Some(error.to_string()),
                    }
                }
            };
            results.push(outcome);
        }
        results
    }

    /// Download a single image candidate to its configured local path.
    pub async fn download_one(&self, image: &ImageRef) -> Result<PathBuf, ImageError> {
        let url = image
            .source_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .ok_or(ImageError::Empty)?;
        self.fetch(image, url).await
    }

    async fn fetch(&self, image: &ImageRef, url: &str) -> Result<PathBuf, ImageError> {
        let mut last_error = None;
        for _ in 0..3 {
            match self.client.get(url).send().await {
                Ok(response) => {
                    let bytes = response.bytes().await.map_err(ImageError::Download)?;
                    if bytes.is_empty() {
                        return Err(ImageError::Empty);
                    }
                    let target = resolve_target(image, &image.local_path);
                    return write_image(&bytes, &target, max_width(&self.config, image.kind));
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(ImageError::Download(
            last_error.expect("at least one attempt"),
        ))
    }
}

fn max_width(config: &ImageConfig, kind: ImageKind) -> u32 {
    match kind {
        ImageKind::Fanart | ImageKind::Banner => config.max_backdrop_width,
        _ => config.max_poster_width,
    }
}

/// Choose the on-disk path. The NFO reader already supplies Kodi-style paths
/// (poster.jpg, .actors/<name>.jpg, ...); fall back to a kind-based name.
fn resolve_target(image: &ImageRef, proposed: &Path) -> PathBuf {
    if proposed.as_os_str().is_empty() {
        PathBuf::from(default_name(image.kind))
    } else {
        proposed.to_path_buf()
    }
}

fn default_name(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Poster => "poster.jpg",
        ImageKind::Fanart => "fanart.jpg",
        ImageKind::Banner => "banner.jpg",
        ImageKind::SeasonPoster => "season-poster.jpg",
        ImageKind::EpisodeThumb => "thumb.jpg",
        ImageKind::Actor => "actor.jpg",
        ImageKind::Logo => "logo.png",
        ImageKind::ClearArt => "clearart.png",
    }
}

/// Decode, resize and save a downloaded image. Kept separate from networking
/// so it can be tested without touching the network.
pub fn write_image(bytes: &[u8], target: &Path, max_width: u32) -> Result<PathBuf, ImageError> {
    let decoded = image::load_from_memory(bytes)?;
    let (width, height) = decoded.dimensions();
    let scaled = if width > max_width {
        let new_height = height * max_width / width;
        decoded.resize(max_width, new_height, FilterType::Lanczos3)
    } else {
        decoded
    };

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ImageError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let temporary = target.with_extension("tmp");
    let result = if target.extension().and_then(|ext| ext.to_str()) == Some("png") {
        scaled.save_with_format(&temporary, image::ImageFormat::Png)
    } else {
        use image::codecs::jpeg::JpegEncoder;
        let mut file = std::fs::File::create(&temporary).map_err(|source| ImageError::Io {
            path: temporary.clone(),
            source,
        })?;
        let mut encoder = JpegEncoder::new_with_quality(&mut file, 88);
        encoder.encode_image(&scaled.to_rgb8())
    };
    result.map_err(ImageError::Decode)?;
    std::fs::rename(&temporary, target).map_err(|source| ImageError::Io {
        path: target.to_path_buf(),
        source,
    })?;
    Ok(target.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgb, RgbImage};

    #[test]
    fn writes_and_resizes_a_downloaded_image() {
        let source = RgbImage::from_fn(120, 60, |x, y| Rgb::from([x as u8, y as u8, 100]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(source)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("poster.jpg");
        write_image(&bytes, &target, 60).unwrap();

        let saved = image::open(&target).unwrap();
        assert_eq!(saved.dimensions(), (60, 30));
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let source = RgbImage::new(20, 10);
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(source)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("thumb.jpg");
        write_image(&bytes, &target, 1000).unwrap();
        assert_eq!(image::open(&target).unwrap().dimensions(), (20, 10));
    }

    #[test]
    fn backdrops_get_a_larger_width_limit() {
        let config = ImageConfig::default();
        assert_eq!(max_width(&config, ImageKind::Fanart), 1920);
        assert_eq!(max_width(&config, ImageKind::Poster), 1000);
    }
}
