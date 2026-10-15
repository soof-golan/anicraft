//! Exports a large animation to check that the exporter stays linear: 600 frames of 200 moving shapes, a quarter of
//! them in groups. The time and size are printed (`cargo test -- --nocapture`) and bounded generously.

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::cast_precision_loss, clippy::as_conversions, clippy::suboptimal_flops)]

use std::time::{Duration, Instant};

use anicraft_swf::export::{ExportOptions, export_display_lists};
use anim_core::kurbo::{BezPath, Circle, Shape};
use anim_core::{Affine, Rgba8, Size};
use anim_display::{DisplayList, Fill, FillRule, Group, ImageStore, Node, Paint};

fn frame(t: u32) -> DisplayList {
    let blob: BezPath = Circle::new((0.0, 0.0), 12.0).to_path(0.05);
    let nodes = (0..200_u32)
        .map(|i| {
            let x = f64::from(i % 20) * 24.0 + f64::from(t % 40);
            let y = f64::from(i / 20) * 24.0 + f64::from(t % 7);
            let grouped = i % 4 == 0;
            // grouped shapes keep their content still and move the group, like symbol instances do
            let shape = Node::Fill(Fill {
                transform: if grouped { Affine::IDENTITY } else { Affine::translate((x, y)) },
                path: blob.clone(),
                rule: FillRule::NonZero,
                paint: Paint::Solid(Rgba8::opaque(u8::try_from(i % 256).unwrap(), 100, 200)),
            });
            if grouped { Node::Group(Group { transform: Affine::translate((x, y)), children: vec![shape], ..Group::default() }) } else { shape }
        })
        .collect();
    DisplayList { size: Size::new(480.0, 240.0), background: Some(Rgba8::WHITE), nodes }
}

#[test]
fn six_hundred_frames_of_two_hundred_shapes() {
    let started = Instant::now();
    let frames: Vec<DisplayList> = (0..600).map(frame).collect();
    let built = started.elapsed();
    let options = ExportOptions::for_list(&frames[0], 30.0);
    let started = Instant::now();
    let out = export_display_lists(&frames, &ImageStore::new(), options).unwrap();
    let exported = started.elapsed();
    eprintln!(
        "built {} lists in {built:?}; exported {} frames, {} characters, {} bytes in {exported:?}",
        frames.len(),
        out.report.frames,
        out.report.characters,
        out.swf.len()
    );
    assert_eq!(out.report.frames, 600);
    // the shapes are shared: one circle per colour, one sprite per colour for the grouped ones
    assert!(out.report.characters <= 300, "{} characters", out.report.characters);
    assert!(exported < Duration::from_secs(60), "export took {exported:?}");
}
