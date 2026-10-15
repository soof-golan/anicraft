//! SWF writer for `AniCraft`.
//!
//! - [`bits`], [`types`], [`tag`]: the bit-packed primitives (`RECT`, `MATRIX`, `CXFORMWITHALPHA`), twips and the tag
//!   header.
//! - [`style`], [`shape`]: fill and line styles and `DefineShape4` bodies, with a builder from `kurbo` paths.
//!
//! This crate only writes SWF. Tests read the output back with Ruffle's `swf` crate.

pub mod bits;
pub mod error;
pub mod shape;
pub mod style;
pub mod tag;
pub mod types;

pub use error::SwfError;
pub use types::CharacterId;
