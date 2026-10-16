//! The integration tests of `anicraft-cli`, built as one test binary (one link instead of five). Each module is a
//! theme; shared helpers live in `common`.

mod common;

mod bridge;
mod cli;
mod mcp;
mod samples;
mod shape_ops;
mod visual_xfl;
