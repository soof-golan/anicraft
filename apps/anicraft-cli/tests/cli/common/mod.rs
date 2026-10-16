//! Helpers shared by the integration tests.
#![allow(dead_code, reason = "each test binary uses a different part")]
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

pub mod client;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The hand-written fixture: a red square tweening from x=10 to x=90 over 12 frames at 12 fps, plus a Hebrew label.
pub fn mover() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mover")
}

/// Runs the binary.
pub fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_anicraft-cli")).args(args).output().unwrap()
}

/// Runs the binary and requires success; returns stdout.
pub fn cli_ok(args: &[&str]) -> String {
    let out = cli(args);
    assert!(out.status.success(), "anicraft-cli {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// A decoded RGBA8 image.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    pub fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = usize::try_from((y * self.width + x) * 4).unwrap();
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Pixels that are clearly red (the fixture's square).
    pub fn red_columns(&self) -> Vec<u32> {
        (0..self.width).filter(|&x| (0..self.height).any(|y| matches!(self.at(x, y), [r, g, b, _] if r > 200 && g < 60 && b < 60))).collect()
    }
}

/// Decodes PNG bytes into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Pixels {
    let png = anim_testkit::decode_png(bytes).unwrap();
    Pixels { width: png.width, height: png.height, rgba: png.rgba }
}

pub fn read_png(path: &Path) -> Pixels {
    decode_png(&std::fs::read(path).unwrap())
}
