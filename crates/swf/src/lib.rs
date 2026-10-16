#![doc = include_str!("../README.md")]

pub mod bitmap;
pub mod bits;
pub mod button;
pub mod edge;
pub mod error;
pub mod export;
pub mod filter;
pub mod morph;
pub mod movie;
pub mod place;
pub mod script;
pub mod shape;
mod shape_cache;
pub mod sound;
pub mod style;
pub mod tag;
pub mod types;

pub use error::SwfError;
pub use export::{DisplayListExporter, ExportNote, ExportOptions, ExportOutput, ExportReport, NodeConverter, Stage, export_display_lists, export_still};
pub use movie::{Compression, Frame, FrameItem, Movie, MovieSettings};
pub use place::Placement;
pub use types::CharacterId;
