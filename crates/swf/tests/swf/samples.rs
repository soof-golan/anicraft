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
use anim_core::{Affine, Rgba8};
use anim_display::{Cap, Join, Spread};
use anim_shape::{BitmapFill, EdgeShape, FillStyle, GradientFill, GradientStop, OtherStroke, SolidStroke, StrokeKind, StrokeStyle, XflEdge, XflName};

use crate::common;

fn samples_dir() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from)
}

fn sample_files() -> Vec<PathBuf> {
    let Some(dir) = samples_dir() else { return Vec::new() };
    ["unicorn_4AE.fla", "derwood_green_choreo_short.fla"].into_iter().map(|n| dir.join(n)).filter(|p| p.exists()).collect()
}

/// The library XML documents of an FLA (the central directory of Animate's zips is 54 bytes short; anim-archive
/// copes).
fn library_xml(path: &Path) -> Vec<(String, String)> {
    anim_testkit::archive_texts(path, |name| Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) && name.starts_with("LIBRARY/"))
        .unwrap()
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn first_element<'a, 'i>(node: roxmltree::Node<'a, 'i>) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(roxmltree::Node::is_element)
}

fn number(node: roxmltree::Node<'_, '_>, name: &str, default: f64) -> f64 {
    node.attribute(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn color(node: roxmltree::Node<'_, '_>) -> Rgba8 {
    let base = Rgba8::from_hex(node.attribute("color").unwrap_or("#000000")).unwrap_or(Rgba8::BLACK);
    base.with_alpha_f32(anim_core::num::f64_to_f32(number(node, "alpha", 1.0)))
}

fn matrix(node: roxmltree::Node<'_, '_>) -> Affine {
    let Some(m) = child(node, "matrix").and_then(|m| child(m, "Matrix")) else { return Affine::IDENTITY };
    Affine::new([number(m, "a", 1.0), number(m, "b", 0.0), number(m, "c", 0.0), number(m, "d", 1.0), number(m, "tx", 0.0), number(m, "ty", 0.0)])
}

fn fill_style(node: roxmltree::Node<'_, '_>) -> FillStyle {
    match node.tag_name().name() {
        "LinearGradient" | "RadialGradient" => {
            let gradient = GradientFill {
                matrix: matrix(node),
                stops: node
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "GradientEntry")
                    .map(|e| GradientStop { offset: anim_core::num::f64_to_f32(number(e, "ratio", 0.0)), color: color(e) })
                    .collect(),
                spread: node.attribute("spreadMethod").and_then(Spread::from_xfl).unwrap_or(Spread::Pad),
                interpolation: node.attribute("interpolationMethod").and_then(anim_shape::Interpolation::from_xfl).unwrap_or_default(),
                focal: number(node, "focalPointRatio", 0.0),
            };
            if node.tag_name().name() == "LinearGradient" { FillStyle::LinearGradient(gradient) } else { FillStyle::RadialGradient(gradient) }
        }
        "BitmapFill" => FillStyle::Bitmap(BitmapFill {
            path: node.attribute("bitmapPath").unwrap_or_default().to_owned(),
            matrix: matrix(node),
            clipped: node.attribute("bitmapIsClipped") == Some("true"),
            smoothing: true,
        }),
        _ => FillStyle::Solid(color(node)),
    }
}

fn stroke_style(node: roxmltree::Node<'_, '_>) -> StrokeStyle {
    let base = SolidStroke {
        weight: number(node, "weight", 1.0),
        fill: child(node, "fill").and_then(first_element).map_or_else(FillStyle::default, fill_style),
        caps: node.attribute("caps").and_then(Cap::from_xfl).unwrap_or(Cap::Round),
        joints: node.attribute("joints").and_then(Join::from_xfl).unwrap_or(Join::Round),
        miter_limit: number(node, "miterLimit", 3.0),
        scale_mode: node.attribute("scaleMode").and_then(anim_shape::StrokeScale::from_xfl).unwrap_or_default(),
        pixel_hinting: node.attribute("pixelHinting") == Some("true"),
    };
    match StrokeKind::from_element(node.tag_name().name()) {
        Some(kind) => StrokeStyle::Other(OtherStroke { kind, base, attrs: node.attributes().map(|a| (a.name().to_owned(), a.value().to_owned())).collect() }),
        None => StrokeStyle::Solid(base),
    }
}

/// All `DOMShape`s of one XML document (damaged edges are kept without geometry).
fn shapes(xml: &str) -> Vec<EdgeShape> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "DOMShape")
        .map(|shape| {
            let indexed = |container: &str, item: &str| -> Vec<roxmltree::Node<'_, '_>> {
                let mut found: Vec<(usize, roxmltree::Node<'_, '_>)> = child(shape, container)
                    .map(|c| {
                        c.children()
                            .filter(|n| n.is_element() && n.tag_name().name() == item)
                            .filter_map(|n| Some((n.attribute("index")?.parse().ok()?, n)))
                            .collect()
                    })
                    .unwrap_or_default();
                found.sort_by_key(|(i, _)| *i);
                found.into_iter().map(|(_, n)| n).collect()
            };
            let fills: Vec<FillStyle> =
                indexed("fills", "FillStyle").into_iter().map(|f| first_element(f).map_or_else(FillStyle::default, fill_style)).collect();
            let strokes: Vec<StrokeStyle> =
                indexed("strokes", "StrokeStyle").into_iter().map(|s| first_element(s).map_or_else(StrokeStyle::default, stroke_style)).collect();
            let edges: Vec<XflEdge> = child(shape, "edges")
                .map(|e| {
                    e.children()
                        .filter(|n| n.is_element() && n.tag_name().name() == "Edge")
                        .map(|n| XflEdge {
                            fill_style0: n.attribute("fillStyle0").and_then(|v| v.parse().ok()).unwrap_or(0),
                            fill_style1: n.attribute("fillStyle1").and_then(|v| v.parse().ok()).unwrap_or(0),
                            stroke_style: n.attribute("strokeStyle").and_then(|v| v.parse().ok()).unwrap_or(0),
                            edges: n.attribute("edges").map(str::to_owned),
                            cubics: n.attribute("cubics").map(str::to_owned),
                        })
                        .collect()
                })
                .unwrap_or_default();
            EdgeShape::from_xfl_lenient(fills, strokes, &edges).0
        })
        .collect()
}

fn all_shapes(path: &Path) -> Vec<EdgeShape> {
    library_xml(path).iter().flat_map(|(_, xml)| shapes(xml)).collect()
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
