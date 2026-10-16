//! The integration tests of `anicraft-tools`, built as one test binary (one link instead of six). Each module is a
//! theme; shared helpers live in `common`.

mod common;

mod drawing;
mod mcp;
mod painting;
mod scripts;
mod selecting;
mod styling;
mod text;
