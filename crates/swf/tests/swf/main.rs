//! The integration tests of `anicraft-swf`, built as one test binary (one link instead of nine). Each module is a
//! theme; shared helpers live in `common`.

mod common;

mod armature;
mod edge_shapes;
mod morph;
mod proptests;
mod readback;
mod render_oracle;
mod samples;
mod scale;
mod script;
mod symbol_mode;
mod visual;
