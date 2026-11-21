//! Media library discovery and filename parsing.

pub mod filenames;
pub mod scanner;

pub use filenames::{
    episode_numbers, format_name, season_number, stacked_base_name, without_extension,
};
pub use scanner::{ScanConfig, ScanResult, ScanRoot, Scanner};
