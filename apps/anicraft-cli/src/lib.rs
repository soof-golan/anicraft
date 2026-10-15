//! Headless `AniCraft`.
//!
//! The binary `anicraft-cli` is a thin shell over this library, so the same code serves the command line, the MCP
//! server (`anicraft-cli mcp`) and tests:
//!
//! * [`project`] opens a document (`anicraft-xfl`) and builds timeline evaluators (`anim-timeline`) that decode its
//!   bitmaps.
//! * [`export`] renders frames (`anim-render`) and writes PNG sequences, GIF, MP4/MOV (`anim-video`), SVG, sprite
//!   sheets (`anim-export`) and SWF (`anicraft-swf`).
//! * [`audio`] mixes the sound layers for video (`anim-audio`).
//! * [`info`] and [`convert`] describe and re-write documents.
//! * [`mcp`] exposes the same operations, plus editing commands, as an MCP server (`anim-mcp`).
//!
//! Frame numbers at every user-facing edge are 1-based like Animate's timeline; the engine counts from 0.

pub mod audio;
pub mod cli;
pub mod convert;
pub mod error;
pub mod export;
pub mod info;
pub mod mcp;
pub mod project;

pub use error::{CliError, Result};
