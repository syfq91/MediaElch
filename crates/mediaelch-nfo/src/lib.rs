//! Kodi NFO sidecar reading and writing.

pub mod reader;
pub mod writer;

pub use reader::{NfoError, NfoReader};
pub use writer::{KodiVersion, NfoWriter};
