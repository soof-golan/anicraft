//! Converts the shapes of the Adobe-authored sample files to SWF. The samples are never committed: set
//! `ANICRAFT_SAMPLES` to the sample folder to run these, otherwise every test returns at once.
//!
//! `sample_shapes_convert_to_swf_and_parse` checks that every library shape converts, encodes and parses back.
//! `sample_shape_contact_sheet` additionally needs `ANICRAFT_RUFFLE_EXPORTER` and `ANICRAFT_SWF_PREVIEW` (an output
//! folder) and renders the most complex shapes, as SWF, with Ruffle into a PNG for a person to look at.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::suboptimal_flops
)]

use std::path::{Path, PathBuf};

use anicraft_swf::edge::{NoBitmaps, shape_from_edges};
use anicraft_swf::movie::{Compression, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::place::Placement;
use anicraft_swf::types::Matrix;
use anicraft_xfl::ReadOptions;
use anim_core::{Affine, Rgba8};
use anim_scene::Element;
use anim_shape::EdgeShape;

use crate::common;

fn sample_files() -> Vec<PathBuf> {
    ["unicorn_4AE.fla", "derwood_green_choreo_short.fla"].into_iter().filter_map(anim_testkit::sample).collect()
}

/// Every shape of every library symbol, groups opened up (damaged edges are kept without geometry).
fn all_shapes(path: &Path) -> Vec<EdgeShape> {
    fn collect(elements: &[Element], out: &mut Vec<EdgeShape>) {
        for element in elements {
            match element {
                Element::Shape(shape) => out.push(shape.shape.clone()),
                Element::Group(group) => collect(&group.members, out),
                _ => {}
            }
        }
    }
    let loaded = anicraft_xfl::read_with(path, &ReadOptions { load_media: false, ..ReadOptions::default() }).unwrap();
    let mut shapes = Vec::new();
    for symbol in loaded.document.library.symbols() {
        for layer in &symbol.timeline.layers {
            for frame in &layer.frames {
                collect(&frame.elements, &mut shapes);
            }
        }
    }
    shapes
}

#[test]
fn sample_shapes_convert_to_swf_and_parse() {
    for path in sample_files() {
        let shapes = all_shapes(&path);
        // the unicorn sample has a library of shapes; the dance sample keeps its drawings inside symbols' timelines
        if shapes.is_empty() {
            eprintln!("{}: no library shapes", path.display());
            continue;
        }
        let mut defined = 0_usize;
        for chunk in shapes.chunks(3000) {
            let mut movie = Movie::new(MovieSettings { compression: Compression::Zlib, ..MovieSettings::default() }).unwrap();
            let mut items = Vec::new();
            for (index, shape) in chunk.iter().enumerate() {
                let converted = shape_from_edges(shape, &NoBitmaps).unwrap();
                let id = movie.define_shape(&converted.shape).unwrap();
                items.push(FrameItem::Place(Placement::new(u16::try_from(index + 1).unwrap(), id, Matrix::IDENTITY)));
                defined += 1;
            }
            movie.add_frame(&Frame::new(items)).unwrap();
            let swf = movie.finish().unwrap();
            let buf = swf::decompress_swf(&swf[..]).unwrap();
            let parsed = swf::parse_swf(&buf).unwrap();
            let shape_tags = parsed.tags.iter().filter(|t| matches!(t, swf::Tag::DefineShape(_))).count();
            assert!(shape_tags > 0 && shape_tags <= chunk.len());
        }
        assert_eq!(defined, shapes.len());
        eprintln!("{}: converted {defined} shapes", path.display());
    }
}

#[test]
fn sample_shape_contact_sheet() {
    let Some(preview) = std::env::var_os("ANICRAFT_SWF_PREVIEW").map(PathBuf::from) else { return };
    if common::exporter().is_none() {
        return;
    }
    for path in sample_files() {
        let mut shapes = all_shapes(&path);
        if shapes.is_empty() {
            continue;
        }
        // the most complex shapes first, as they show the most
        shapes.sort_by_key(|s| std::cmp::Reverse(s.edges.iter().map(|e| e.segments.len()).sum::<usize>()));
        shapes.truncate(96);
        let (columns, cell) = (12_usize, 120.0_f64);
        let rows = shapes.len().div_ceil(columns);
        let mut movie = Movie::new(MovieSettings {
            width: cell * 12.0,
            height: cell * f64::from(u16::try_from(rows).unwrap()),
            background: Rgba8::opaque(235, 235, 240),
            compression: Compression::None,
            ..MovieSettings::default()
        })
        .unwrap();
        let mut items = Vec::new();
        for (index, shape) in shapes.iter().enumerate() {
            let converted = shape_from_edges(shape, &NoBitmaps).unwrap();
            let bounds = converted.shape.edge_bounds;
            let (w, h) = (f64::from(bounds.x_max - bounds.x_min) / 20.0, f64::from(bounds.y_max - bounds.y_min) / 20.0);
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            let scale = ((cell - 12.0) / w).min((cell - 12.0) / h).min(4.0);
            let (column, row) = (f64::from(u16::try_from(index % columns).unwrap()), f64::from(u16::try_from(index / columns).unwrap()));
            let tx = column * cell + (cell - w * scale) / 2.0 - f64::from(bounds.x_min) / 20.0 * scale;
            let ty = row * cell + (cell - h * scale) / 2.0 - f64::from(bounds.y_min) / 20.0 * scale;
            let id = movie.define_shape(&converted.shape).unwrap();
            items.push(FrameItem::Place(Placement::new(
                u16::try_from(index + 1).unwrap(),
                id,
                Matrix::from_affine(Affine::new([scale, 0.0, 0.0, scale, tx, ty])),
            )));
        }
        movie.add_frame(&Frame::new(items)).unwrap();
        let swf = movie.finish().unwrap();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let rendered = common::render_swf(&format!("sheet_{stem}"), &swf).unwrap();
        // keep the SWF and the render for people to look at
        std::fs::create_dir_all(&preview).unwrap();
        std::fs::write(preview.join(format!("anicraft-swf-{stem}-shapes.swf")), &swf).unwrap();
        let png_source = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("render_oracle").join(format!("sheet_{stem}.png"));
        std::fs::copy(png_source, preview.join(format!("anicraft-swf-{stem}-shapes.png"))).unwrap();
        assert!(rendered.width > 0);
    }
}
