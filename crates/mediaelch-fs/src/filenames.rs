//! Filename parsing rules ported from MediaElch's filename utilities and
//! TV show file searcher. These are the golden cases from the original Catch2
//! suite (test/unit/file, test/unit/media, test/unit/tv_shows).

use std::sync::OnceLock;

use regex::Regex;

fn stacked_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^(.*)([ _.-]+(?:cd|dvd|pt|part|dis[ck])[ _.-]*[0-9a-f]+)(.*)(\.[^.]+)$")
            .unwrap()
    })
}

fn trailing_separator_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(.*)([ _.-]+)$").unwrap())
}

fn sxe_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)s(\d+)[ ._-]*x?[ ._-]*e(\d+)").unwrap())
}

fn chained_e_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^((?:e\d+)+)").unwrap())
}

fn separated_e_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^[ ._-]+e(\d+)").unwrap())
}

fn e_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)e(\d+)").unwrap())
}

fn nx_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+)x(\d+)").unwrap())
}

fn chained_x_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^x(\d+)").unwrap())
}

fn episode_word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)episode[ ._-]*(\d+)").unwrap())
}

fn ep_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)ep[ ._-]*(\d+)").unwrap())
}

fn separated_number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[ ._-]+(\d{1,4})(?:[^0-9]|$)").unwrap())
}

fn season_word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)season[ ._-]*(\d+)").unwrap())
}

fn dotted_numbers_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^0-9])(\d{1,3})\.(\d{2,4})(?:[^0-9]|$)").unwrap())
}

/// Remove a `cd1`, `part2`, `pt3`, `disc4`, ... stack suffix from a filename.
/// If there is no stack suffix the input is returned unchanged.
pub fn stacked_base_name(file_name: &str) -> String {
    let Some(caps) = stacked_re().captures(file_name) else {
        return file_name.to_owned();
    };
    let mut title = caps.get(1).unwrap().as_str().to_owned();
    while let Some(caps) = trailing_separator_re().captures(&title) {
        let next = caps.get(1).unwrap().as_str().to_owned();
        if next == title {
            break;
        }
        title = next;
    }
    title
}

/// Remove only the final extension. `nodot` stays `nodot`.
pub fn without_extension(file_name: &str) -> &str {
    match file_name.rfind('.') {
        Some(index) if index > 0 => &file_name[..index],
        _ => file_name,
    }
}

/// Extract all episode numbers from a filename, in the order they appear.
pub fn episode_numbers(input: &str) -> Vec<u32> {
    let mut numbers = Vec::new();

    for caps in sxe_re().captures_iter(input) {
        push_unique(&mut numbers, parse_group(&caps, 2));
        let rest = &input[caps.get(0).unwrap().end()..];
        if let Some(chain) = chained_e_re().captures(rest) {
            for e in e_number_re().captures_iter(chain.get(1).unwrap().as_str()) {
                push_unique(&mut numbers, parse_group(&e, 1));
            }
        } else if let Some(extra) = separated_e_re().captures(rest) {
            push_unique(&mut numbers, parse_group(&extra, 1));
        }
    }

    for caps in nx_re().captures_iter(input) {
        push_unique(&mut numbers, parse_group(&caps, 2));
        let rest = &input[caps.get(0).unwrap().end()..];
        // Only the first chained token is contiguous with the `NxNN` match.
        if let Some(x) = chained_x_re().captures(rest) {
            push_unique(&mut numbers, parse_group(&x, 1));
        }
    }

    for caps in episode_word_re().captures_iter(input) {
        push_unique(&mut numbers, parse_group(&caps, 1));
    }

    for caps in ep_re().captures_iter(input) {
        push_unique(&mut numbers, parse_group(&caps, 1));
        let rest = &input[caps.get(0).unwrap().end()..];
        if let Some(extra) = separated_number_re().captures(rest) {
            push_unique(&mut numbers, parse_group(&extra, 1));
        }
    }

    if numbers.is_empty() {
        if let Some(caps) = dotted_numbers_re().captures(input) {
            push_unique(&mut numbers, parse_group(&caps, 2));
        }
    }

    numbers
}

/// Extract the season number from a filename, following the same priority as
/// the original `TvShowFileSearcher::getSeasonNumber`.
pub fn season_number(input: &str) -> Option<u32> {
    if let Some(caps) = sxe_re().captures(input) {
        return Some(parse_group(&caps, 1));
    }
    if let Some(caps) = nx_re().captures(input) {
        return Some(parse_group(&caps, 1));
    }
    if let Some(caps) = season_word_re().captures(input) {
        return Some(parse_group(&caps, 1));
    }
    dotted_numbers_re()
        .captures(input)
        .map(|caps| parse_group(&caps, 1))
}

/// Make a filesystem-derived title readable by replacing separators with
/// spaces. Exclude-word stripping is configuration-driven in the original and
/// is handled by the scanner's exclude list.
pub fn format_name(input: &str) -> String {
    input
        .replace(['_', '.'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_group(caps: &regex::Captures<'_>, index: usize) -> u32 {
    caps.get(index)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0)
}

fn push_unique(numbers: &mut Vec<u32>, number: u32) {
    if number > 0 && !numbers.contains(&number) {
        numbers.push(number);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_base_name_matches_original_golden_cases() {
        assert_eq!(stacked_base_name("movie-cd1.avi"), "movie");
        assert_eq!(stacked_base_name("movie-cd2.avi"), "movie");
        assert_eq!(stacked_base_name("movie-part1.mkv"), "movie");
        assert_eq!(stacked_base_name("movie-CD1.avi"), "movie");
        assert_eq!(stacked_base_name("movie-Part1.mkv"), "movie");
        assert_eq!(stacked_base_name("movie.mkv"), "movie.mkv");
        assert_eq!(
            stacked_base_name("The.Dark.Knight.mkv"),
            "The.Dark.Knight.mkv"
        );
        assert_eq!(
            stacked_base_name("/path/to/movie.mkv/captain.america.part1.mk4"),
            "/path/to/movie.mkv/captain.america"
        );
        assert_eq!(
            stacked_base_name("/path/to/movie.mkv/captain.marvel.pt1.ignored.mk4"),
            "/path/to/movie.mkv/captain.marvel"
        );
        assert_eq!(
            stacked_base_name(r"C:\path\to\movie.mkv\captain.america.part-1.mk4"),
            r"C:\path\to\movie.mkv\captain.america"
        );
    }

    #[test]
    fn without_extension_matches_original_golden_cases() {
        assert_eq!(without_extension("foo.mkv"), "foo");
        assert_eq!(without_extension("my.movie.mkv"), "my.movie");
        assert_eq!(without_extension("nodot"), "nodot");
    }

    #[test]
    fn episode_numbers_match_original_golden_cases() {
        let cases = [
            ("dir/S01E04.mov", vec![4]),
            ("dir/S01E004.mov", vec![4]),
            ("dir/S1E1425.mov", vec![1425]),
            ("dir/S01.E04.mov", vec![4]),
            ("dir/S01_E04.mov", vec![4]),
            ("dir/S01xE04.mov", vec![4]),
            ("dir/01x004.mov", vec![4]),
            ("dir/Season.01-Episode.142.mov", vec![142]),
            ("dir/Name_before_S01E142.mov", vec![142]),
            ("dir/S01E14_Name.mov", vec![14]),
            ("Oz/Oz.S01E01.Emerald City (720p)", vec![1]),
            ("dir/ep004.mov", vec![4]),
            ("dir/Name with space ep_14.mov", vec![14]),
            ("dir/ep14_Name.mov", vec![14]),
            ("dir/4.12.mov", vec![12]),
            ("dir/S01E4-S01E5.mov", vec![4, 5]),
            ("dir/S01E004-S01E005-S01E15.mov", vec![4, 5, 15]),
            ("dir/S01E02E03E04E06.mov", vec![2, 3, 4, 6]),
            ("dir/Name S01E04-E6.mov", vec![4, 6]),
            ("dir/Name s01e01-s01e02-s01e03.mov", vec![1, 2, 3]),
            ("dir/Name 1x01x02.mov", vec![1, 2]),
            ("dir/ep4-ep5.mov", vec![4, 5]),
            ("dir/ep04-05.mov", vec![4, 5]),
        ];
        for (input, expected) in cases {
            assert_eq!(episode_numbers(input), expected, "input: {input}");
        }
    }

    #[test]
    fn season_numbers_match_original_golden_cases() {
        let cases = [
            ("dir/4.12.mov", 4),
            ("dir/S004E001.mov", 4),
            ("dir/S1425E1005.mov", 1425),
            ("dir/Season.11-Episode.1.mov", 11),
            ("dir/Season14Episode16.mov", 14),
            ("dir/Name_before_S142E2.mov", 142),
            ("Oz/Oz.S04E01.Emerald City (720p)", 4),
            ("dir/S004E004-S004E005-S04E15.mov", 4),
            ("dir/S04E02E03E04E06.mov", 4),
        ];
        for (input, expected) in cases {
            assert_eq!(season_number(input), Some(expected), "input: {input}");
        }
    }

    #[test]
    fn format_name_replaces_separators_with_spaces() {
        assert_eq!(format_name("The.Matrix_1999"), "The Matrix 1999");
    }
}
