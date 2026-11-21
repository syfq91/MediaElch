//! Server-rendered HTML views for the headless MediaElch web UI.

use askama::Template;
use mediaelch_core::MediaItem;

pub const APP_CSS: &str = include_str!("../static/app.css");

#[derive(Debug, Clone)]
pub struct ItemCard {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub has_poster: bool,
    pub poster_url: String,
}

#[derive(Debug, Template)]
#[template(path = "library.html")]
pub struct LibraryView {
    pub items: Vec<ItemCard>,
    pub total: i64,
}

#[derive(Debug, Template)]
#[template(path = "item.html")]
pub struct ItemView {
    pub item: MediaItem,
}

#[derive(Debug, Clone)]
pub struct ScanJobView {
    pub id: String,
    pub status: String,
    pub processed: usize,
    pub total: usize,
    pub error: Option<String>,
}

#[derive(Debug, Template)]
#[template(path = "tools.html")]
pub struct ToolsView;

#[derive(Debug, Template)]
#[template(path = "scan_status.html")]
pub struct ScanStatusView {
    pub job: ScanJobView,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_view_renders_cards_and_placeholders() {
        let view = LibraryView {
            items: vec![ItemCard {
                id: 7,
                kind: "movie".to_owned(),
                title: "Alien".to_owned(),
                subtitle: "1979".to_owned(),
                has_poster: false,
                poster_url: String::new(),
            }],
            total: 1,
        };
        let html = view.render().unwrap();
        assert!(html.contains("Alien"));
        assert!(html.contains("/ui/items/7"));
        assert!(html.contains("1 item(s)"));
        assert!(html.contains("htmx"));
    }

    #[test]
    fn scan_status_view_shows_progress() {
        let html = ScanStatusView {
            job: ScanJobView {
                id: "scan-1".to_owned(),
                status: "running".to_owned(),
                processed: 3,
                total: 10,
                error: None,
            },
        }
        .render()
        .unwrap();
        assert!(html.contains("scan-1"));
        assert!(html.contains("3/10"));
    }
}
