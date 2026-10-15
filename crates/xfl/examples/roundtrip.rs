//! Reads a document, writes it again, reads that back and reports what differs.
//!
//! `cargo run -p anicraft-xfl --example roundtrip -- FILE OUT [--folder]`
//!
//! `OUT` is a `.fla` (or a folder with `--folder`). The report lists whether the two models are equal, how the two
//! `DOMDocument.xml` texts differ semantically, and which archive entries differ.

#![allow(clippy::float_cmp, reason = "developer tool: models are compared exactly on purpose")]

use std::path::PathBuf;
use std::time::Instant;

use anicraft_xfl::diff::{DiffOptions, compare_archives};
use anicraft_xfl::{ReadOptions, read_with, write_loaded};
use anim_archive::Container;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().ok_or("usage: roundtrip FILE OUT [--folder]")?);
    let target = PathBuf::from(args.next().ok_or("usage: roundtrip FILE OUT [--folder]")?);

    let started = Instant::now();
    let first = read_with(&source, &ReadOptions::default())?;
    println!("read   {} in {:.2?}", source.display(), started.elapsed());

    let started = Instant::now();
    let report = write_loaded(&target, &first)?;
    println!(
        "wrote  {} in {:.2?}: {} entries ({} symbols, {} media), {} bytes, {} warnings",
        target.display(),
        started.elapsed(),
        report.entries,
        report.symbols,
        report.media_entries,
        report.bytes,
        report.warnings.len()
    );
    for w in report.warnings.iter().take(10) {
        println!("  {w}");
    }

    let second = read_with(&target, &ReadOptions::default())?;
    println!("models equal: document {}, media {}", first.document == second.document, first.media == second.media);
    println!("warnings on re-read: {}", second.warnings.len());
    if first.document != second.document {
        for (index, (a, b)) in first.document.scenes.iter().zip(&second.document.scenes).enumerate() {
            if a != b {
                println!("  scene {index} differs");
            }
        }
        for (a, b) in first.document.library.items.iter().zip(&second.document.library.items) {
            if a != b {
                println!("  library item {:?} differs", a.name());
            }
        }
        println!(
            "  doc fields: size {} rate {} bg {} current {} extra {} publish {} library len {}/{}",
            (first.document.width, first.document.height) == (second.document.width, second.document.height),
            first.document.frame_rate == second.document.frame_rate,
            first.document.background_color == second.document.background_color,
            first.document.current_timeline == second.document.current_timeline,
            first.document.extra == second.document.extra,
            first.document.publish_settings_xml == second.document.publish_settings_xml,
            first.document.library.items.len(),
            second.document.library.items.len(),
        );
    }

    let original = Container::open(&source)?;
    let written = Container::open(&target)?;
    let diff = compare_archives(&original, &written, &DiffOptions::default());
    println!("DOMDocument.xml: {} semantic differences", diff.document.total);
    for (key, n) in diff.document.top(15) {
        println!("  {n:6}  {key}");
    }
    for d in diff.document.differences.iter().take(8) {
        println!("  at {} [{:?}] {}", d.path, d.kind, d.detail);
    }
    println!("symbol files that differ: {}", diff.symbols.len());
    for (name, d) in diff.symbols.iter().take(5) {
        println!("  {name}: {} differences", d.total);
        for (key, n) in d.top(4) {
            println!("      {n:5}  {key}");
        }
        for x in d.differences.iter().take(3) {
            println!("      at {} [{:?}] {}", x.path, x.kind, x.detail);
        }
    }
    println!("only in original: {:?}", diff.only_in_first);
    println!("only in written: {:?}", diff.only_in_second);
    println!("bytes differ: {:?}", diff.bytes_differ);
    println!("unreadable: {:?}", diff.unreadable);
    Ok(())
}
