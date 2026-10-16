//! Shared by the tests that render SWFs with Ruffle's `exporter` (see `render_oracle.rs`).

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::path::PathBuf;
use std::process::Command;

/// A rendered frame: RGBA8 pixels, rows top to bottom.
pub struct Rendered {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

impl Rendered {
    pub fn at(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 4;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }

    pub fn assert_near(&self, x: usize, y: usize, expected: [u8; 3], tolerance: i32, what: &str) {
        let got = self.at(x, y);
        let ok = got.iter().zip(expected).all(|(g, e)| (i32::from(*g) - i32::from(e)).abs() <= tolerance);
        assert!(ok, "{what}: pixel ({x}, {y}) is {got:?}, expected about {expected:?}");
    }
}

/// The renderer to use, from `ANICRAFT_RUFFLE_EXPORTER`.
pub fn exporter() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_RUFFLE_EXPORTER").map(PathBuf::from)
}

fn decode(path: &std::path::Path) -> Rendered {
    let decoder = png::Decoder::new(std::io::Cursor::new(std::fs::read(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    Rendered { width: usize::try_from(info.width).unwrap(), height: usize::try_from(info.height).unwrap(), pixels: buf }
}

fn scratch_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("render_oracle");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Renders frame 0 of `swf` and returns its pixels, or `None` when no renderer is configured.
pub fn render_swf(name: &str, swf: &[u8]) -> Option<Rendered> {
    let exporter = exporter()?;
    let dir = scratch_dir();
    let swf_path = dir.join(format!("{name}.swf"));
    let png_path = dir.join(format!("{name}.png"));
    std::fs::write(&swf_path, swf).unwrap();
    let status = Command::new(exporter).arg("-s").arg(&swf_path).arg(&png_path).status().unwrap();
    assert!(status.success(), "exporter failed on {name}");
    Some(decode(&png_path))
}

/// Renders the first `frames` (at most 99) frames of `swf`.
pub fn render_swf_frames(name: &str, swf: &[u8], frames: usize) -> Option<Vec<Rendered>> {
    let exporter = exporter()?;
    let dir = scratch_dir();
    let swf_path = dir.join(format!("{name}.swf"));
    let out_dir = dir.join(format!("{name}_frames"));
    std::fs::write(&swf_path, swf).unwrap();
    let status = Command::new(exporter).arg("-s").arg("-f").arg(frames.to_string()).arg(&swf_path).arg(&out_dir).status().unwrap();
    assert!(status.success(), "exporter failed on {name}");
    // the exporter zero-pads frame numbers to the width of the frame count, which differs between versions
    let frame_file = |i: usize| (1..=3).map(|width| out_dir.join(format!("{i:0width$}.png"))).find(|p| p.exists()).expect("a rendered frame file");
    Some((0..frames).map(|i| decode(&frame_file(i))).collect())
}
