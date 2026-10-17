//! The guides and the grid belong to the document: they survive saving and opening a FLA.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "test code")]

use anicraft_tools::guides::{self, Layout};
use anicraft_xfl::{ReadOptions, WriteOptions, read_bytes, write_fla_bytes};
use anim_core::Rgba8;
use anim_scene::{Document, MediaStore};

fn round_trip(doc: &Document) -> Document {
    let (bytes, _) = write_fla_bytes(doc, &MediaStore::default(), &[], &WriteOptions::default()).unwrap();
    read_bytes(bytes, &ReadOptions::default()).unwrap().document
}

#[test]
fn guides_and_the_grid_round_trip_through_a_fla() {
    let mut doc = Document::default();
    let layout = Layout {
        guides_x: vec![10.0, 250.5],
        guides_y: vec![33.0],
        guides_locked: true,
        guides_visible: false,
        guide_color: Rgba8::opaque(255, 0, 0),
        grid_visible: true,
        grid: [20.0, 12.5],
        grid_color: Rgba8::opaque(1, 2, 3),
    };
    guides::write(&mut doc, &layout);
    assert_eq!(guides::read(&round_trip(&doc)), layout);
}

#[test]
fn a_document_without_guides_stays_free_of_layout_attributes() {
    let loaded = round_trip(&Document::default());
    assert_eq!(guides::read(&loaded), Layout::default());
    assert!(loaded.extra.attr("guidesX").is_none() && loaded.extra.attr("gridVisible").is_none());
}

#[test]
fn clearing_the_guides_removes_them_from_the_file() {
    let mut doc = Document::default();
    guides::write(&mut doc, &Layout { guides_x: vec![5.0], ..Layout::default() });
    let mut layout = guides::read(&round_trip(&doc));
    layout.guides_x.clear();
    let mut reopened = round_trip(&doc);
    guides::write(&mut reopened, &layout);
    assert_eq!(guides::read(&round_trip(&reopened)), Layout::default());
}
