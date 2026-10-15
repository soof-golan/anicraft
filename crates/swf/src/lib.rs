//! SWF writer for `AniCraft`.
//!
//! - [`bits`], [`types`], [`tag`]: the bit-packed primitives (`RECT`, `MATRIX`, `CXFORMWITHALPHA`), twips and the tag
//!   header.
//! - [`style`], [`shape`]: fill and line styles and `DefineShape4` bodies, with a builder from `kurbo` paths.
//! - [`bitmap`], [`filter`], [`place`], [`sound`]: the other tags: bitmaps, filter lists, `PlaceObject3`, sounds.
//! - [`movie`]: [`Movie`], which owns the dictionary and the timeline and writes the file (`FWS` or zlib `CWS`).
//!
//! This crate only writes SWF. Tests read the output back with Ruffle's `swf` crate.

pub mod bitmap;
pub mod bits;
pub mod error;
pub mod filter;
pub mod movie;
pub mod place;
pub mod shape;
pub mod sound;
pub mod style;
pub mod tag;
pub mod types;

pub use error::SwfError;
pub use movie::{Compression, Frame, FrameItem, Movie, MovieSettings};
pub use place::Placement;
pub use types::CharacterId;
