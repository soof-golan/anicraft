//! Prints a summary of an XFL document: library, timelines, unknowns and warnings. Optionally dumps raw media and
//! decoded bitmaps as PNG files.
//!
//! `cargo run -p anicraft-xfl --example inspect -- FILE [DUMP_DIR]`

#![allow(clippy::arithmetic_side_effects, clippy::chunks_exact_to_as_chunks, reason = "developer tool on trusted output of our own decoder")]

use std::path::{Path, PathBuf};

use anicraft_xfl::{ReadOptions, decode_bitmap, read_with};
use anim_scene::LibraryItem;

fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect()
}

fn write_png(path: &Path, image: &anim_display::Image) -> Result<(), Box<dyn std::error::Error>> {
    let mut straight = Vec::with_capacity(image.premultiplied_rgba.len());
    for px in image.premultiplied_rgba.chunks_exact(4) {
        if let [r, g, b, a] = *px {
            let un = |c: u8| if a == 0 { 0 } else { u8::try_from((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).unwrap_or(255) };
            straight.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&straight)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or("usage: inspect FILE [DUMP_DIR]")?);
    let dump = args.next().map(PathBuf::from);
    let loaded = read_with(&path, &ReadOptions::default())?;
    let stats = loaded.document.stats();
    println!("{stats:#?}");
    println!("warnings: {} (+{} dropped)", loaded.warnings.len(), loaded.dropped_warnings);
    for w in loaded.warnings.iter().take(20) {
        println!("  {w}");
    }
    let problems = loaded.document.validate();
    println!("validate: {} problems", problems.len());
    for p in problems.iter().take(15) {
        println!("  {p}");
    }
    println!("unknowns: {} total, {} ui-state", loaded.unknowns.total(), loaded.unknowns.ui_state);
    for (k, n) in loaded.unknowns.top(25) {
        println!("  {n:8}  {k}");
    }
    if let Some(dir) = dump {
        std::fs::create_dir_all(&dir)?;
        for item in &loaded.document.library.items {
            let Some(id) = item.media() else { continue };
            let Some(bytes) = loaded.media.get(id) else { continue };
            std::fs::write(dir.join(format!("{}.bin", safe_name(item.name()))), bytes)?;
            if let LibraryItem::Bitmap(_) = item {
                match decode_bitmap(&loaded.media, id) {
                    Ok(image) => write_png(&dir.join(format!("{}.png", safe_name(item.name()))), &image)?,
                    Err(error) => println!("bitmap {}: {error}", item.name()),
                }
            }
        }
    }
    Ok(())
}
