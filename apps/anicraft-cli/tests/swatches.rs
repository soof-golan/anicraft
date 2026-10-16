//! The document's swatches survive saving and opening a FLA, and the colour commands edit them with undo.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "test code")]

use anicraft_tools::swatches;
use anicraft_xfl::{ReadOptions, WriteOptions, read_bytes, write_fla_bytes};
use anim_core::Rgba8;
use anim_scene::{Document, MediaStore};
use anim_shape::FillStyle;

#[test]
fn swatches_round_trip_through_a_fla() {
    let mut doc = Document::default();
    let mut palette = swatches::default_swatches();
    palette.push(FillStyle::Solid(Rgba8::new(10, 20, 30, 128)));
    swatches::write(&mut doc, &palette);

    let (bytes, _) = write_fla_bytes(&doc, &MediaStore::default(), &[], &WriteOptions::default()).unwrap();
    let loaded = read_bytes(bytes, &ReadOptions::default()).unwrap();
    let back = swatches::read(&loaded.document);
    assert_eq!(back.len(), palette.len());
    assert_eq!(back.first(), palette.first());
    let last = back.last().and_then(FillStyle::solid_color).unwrap();
    assert_eq!((last.r, last.g, last.b), (10, 20, 30));
    assert!((i32::from(last.a) - 128).abs() <= 1);
    // The palette is stored once, not duplicated by the reader's unknown-element bookkeeping.
    assert_eq!(loaded.document.extra.children.iter().filter(|c| c.starts_with("<swatchLists")).count(), 1);
}
