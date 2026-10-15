//! Shared by the tests that render SWFs with Ruffle's `exporter` (see `render_oracle.rs`).

#![allow(dead_code, clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

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

/// Renders frame 0 of `swf` and returns its pixels, or `None` when no renderer is configured.
pub fn render_swf(name: &str, swf: &[u8]) -> Option<Rendered> {
    let exporter = exporter()?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("render_oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let swf_path = dir.join(format!("{name}.swf"));
    let png_path = dir.join(format!("{name}.png"));
    std::fs::write(&swf_path, swf).unwrap();
    let status = Command::new(exporter).arg("-s").arg(&swf_path).arg(&png_path).status().unwrap();
    assert!(status.success(), "exporter failed on {name}");
    let decoder = png::Decoder::new(std::io::Cursor::new(std::fs::read(&png_path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    Some(Rendered { width: usize::try_from(info.width).unwrap(), height: usize::try_from(info.height).unwrap(), pixels: buf })
}
