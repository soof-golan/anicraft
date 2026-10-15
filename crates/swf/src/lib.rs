#![doc = include_str!("../README.md")]

pub mod bitmap;
pub mod bits;
pub mod edge;
pub mod error;
pub mod export;
pub mod filter;
pub mod morph;
pub mod movie;
pub mod place;
pub mod shape;
pub mod sound;
pub mod style;
pub mod tag;
pub mod types;

pub use error::SwfError;
pub use export::{DisplayListExporter, ExportNote, ExportOptions, ExportOutput, ExportReport, export_display_lists, export_still};
pub use movie::{Compression, Frame, FrameItem, Movie, MovieSettings};
pub use place::Placement;
pub use types::CharacterId;
