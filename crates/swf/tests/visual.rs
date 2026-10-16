//! Predict, render, inspect: hand-made hard fixtures for the SWF exporter.
//!
//! The predictions live next to the fixtures in `tests/fixtures/visual/<name>.expect.md` and were written before the
//! first render. Every display-list fixture is exported to SWF and rendered twice: by `anim-render` straight from the
//! display list, and (when `ANICRAFT_RUFFLE_EXPORTER` names a built Ruffle `exporter`) by Ruffle from the SWF. Both are
//! checked against the same probes. PNGs go to `ANICRAFT_VISUAL_OUT` (default `/mnt/data/anim/scratch/visual/swf`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::suboptimal_flops,
    clippy::needless_pass_by_value,
    clippy::print_stderr,
    clippy::type_complexity,
    clippy::too_many_arguments
)]

mod common;

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, TAU};
use std::path::PathBuf;

use anicraft_swf::button::{ButtonDef, ButtonRecord, ButtonStates};
use anicraft_swf::export::{ExportOptions, export_display_lists};
use anicraft_swf::movie::{ACTION_STOP, Compression, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::place::Placement;
use anicraft_swf::shape::{PathStyle, ShapeBuilder};
use anicraft_swf::sound::SoundInfo;
use anicraft_swf::style::FillStyle;
use anicraft_swf::types::{CharacterId, Matrix};
use anim_core::kurbo::{BezPath, Circle, Rect, Shape};
use anim_core::{Affine, BlendMode, ColorTransform, Rgba8, Size};
use anim_display::{
    Cap, DisplayList, Fill, FillRule, Filter, Gradient, GradientStop, Group, ImageStore, Interpolation, Join, Node, Paint, Spread, Stroke, StrokeScale,
    StrokeStyle,
};
use anim_render::RenderOptions;
use common::Rendered;

// ---------------------------------------------------------------------------------------------------------------
// harness

fn out_dir() -> PathBuf {
    let dir = std::env::var_os("ANICRAFT_VISUAL_OUT").map_or_else(|| PathBuf::from("/mnt/data/anim/scratch/visual/swf"), PathBuf::from);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn save(name: &str, image: &Rendered) {
    let path = out_dir().join(format!("{name}.png"));
    let Ok(file) = std::fs::File::create(path) else { return };
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), u32::try_from(image.width).unwrap(), u32::try_from(image.height).unwrap());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let Ok(mut writer) = encoder.write_header() else { return };
    let _ = writer.write_image_data(&image.pixels);
}

/// Compares an `anim-render` picture with the checked-in golden `tests/fixtures/visual/golden/<name>.png` (a few
/// per-channel levels of tolerance, and a tiny share of pixels may differ more). `ANICRAFT_UPDATE_GOLDEN=1` rewrites
/// the golden; a missing golden is skipped.
fn golden(name: &str, image: &Rendered) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visual/golden").join(format!("{name}.png"));
    if std::env::var_os("ANICRAFT_UPDATE_GOLDEN").is_some() {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), u32::try_from(image.width).unwrap(), u32::try_from(image.height).unwrap());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&image.pixels).unwrap();
        return;
    }
    let Ok(file) = std::fs::File::open(&path) else { return };
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    let want = Rendered { width: info.width as usize, height: info.height as usize, pixels: buffer };
    assert_eq!((want.width, want.height), (image.width, image.height), "{name}: golden size");
    let share = differing_share(&want, image, 4);
    assert!(share < 0.002, "{name}: {:.3}% of the pixels differ from the golden {}", share * 100.0, path.display());
}

/// A crop of `image` scaled up with nearest neighbour, for the Read tool.
fn zoom(image: &Rendered, x: usize, y: usize, w: usize, h: usize, scale: usize) -> Rendered {
    let mut pixels = Vec::with_capacity(w * h * scale * scale * 4);
    for oy in 0..h * scale {
        for ox in 0..w * scale {
            let sx = (x + ox / scale).min(image.width - 1);
            let sy = (y + oy / scale).min(image.height - 1);
            let i = (sy * image.width + sx) * 4;
            pixels.extend_from_slice(&image.pixels[i..i + 4]);
        }
    }
    Rendered { width: w * scale, height: h * scale, pixels }
}

/// Both renderings of one display list.
struct Shots {
    ruffle: Option<Rendered>,
    anim: Rendered,
}

fn export_options(list: &DisplayList) -> ExportOptions {
    let mut options = ExportOptions::for_list(list, 24.0);
    options.movie.compression = Compression::None;
    options
}

fn anim_render(list: &DisplayList, images: &ImageStore) -> Rendered {
    let image = anim_render::render(list, images, &RenderOptions::for_stage(list.size)).unwrap();
    Rendered { width: image.width as usize, height: image.height as usize, pixels: image.premultiplied_rgba }
}

fn shoot(name: &str, list: &DisplayList, images: &ImageStore) -> Shots {
    let out = export_display_lists([list], images, export_options(list)).unwrap();
    let ruffle = common::render_swf(name, &out.swf);
    let anim = anim_render(list, images);
    save(&format!("{name}.anim"), &anim);
    golden(name, &anim);
    if let Some(r) = &ruffle {
        save(&format!("{name}.ruffle"), r);
    }
    Shots { ruffle, anim }
}

impl Shots {
    /// Saves nearest-neighbour zooms of the given regions (`label`, x, y, w, h, scale) of both renderings.
    fn crops(&self, name: &str, regions: &[(&str, usize, usize, usize, usize, usize)]) {
        for &(label, x, y, w, h, scale) in regions {
            save(&format!("{name}.anim.{label}"), &zoom(&self.anim, x, y, w, h, scale));
            if let Some(r) = &self.ruffle {
                save(&format!("{name}.ruffle.{label}"), &zoom(r, x, y, w, h, scale));
            }
        }
    }
}

/// Collects every failed probe so a run shows all disagreements at once.
#[derive(Default)]
struct Probes {
    failures: Vec<String>,
}

impl Probes {
    fn check(&mut self, who: &str, label: &str, image: &Rendered, x: usize, y: usize, want: [u8; 3], tolerance: i32) {
        let got = image.at(x, y);
        let ok = got.iter().zip(want).all(|(g, w)| (i32::from(*g) - i32::from(w)).abs() <= tolerance);
        if !ok {
            self.failures.push(format!("[{who}] {label}: pixel ({x}, {y}) is {got:?}, expected about {want:?} (+-{tolerance})"));
        }
    }

    fn both(&mut self, shots: &Shots, label: &str, x: usize, y: usize, want: [u8; 3], tolerance: i32) {
        self.check("anim", label, &shots.anim, x, y, want, tolerance);
        if let Some(r) = &shots.ruffle {
            self.check("ruffle", label, r, x, y, want, tolerance);
        }
    }

    fn require(&mut self, ok: bool, message: String) {
        if !ok {
            self.failures.push(message);
        }
    }

    fn done(self, name: &str) {
        assert!(self.failures.is_empty(), "{name}: {} probes failed:\n{}", self.failures.len(), self.failures.join("\n"));
    }
}

/// The share of pixels that differ by more than `tolerance` in any channel between two renderings.
fn differing_share(a: &Rendered, b: &Rendered, tolerance: i32) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let total = a.width * a.height;
    let bad = (0..total)
        .filter(|i| (0..3).any(|c| (i32::from(a.pixels[i * 4 + c]) - i32::from(b.pixels[i * 4 + c])).abs() > tolerance))
        .count();
    bad as f64 / total as f64
}

// ---------------------------------------------------------------------------------------------------------------
// display list builders

const RED: Rgba8 = Rgba8::opaque(255, 0, 0);
const DARK: [u8; 3] = [30, 30, 30];
const WHITE: [u8; 3] = [255, 255, 255];

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.1)
}

fn poly(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        if i == 0 { path.move_to((x, y)) } else { path.line_to((x, y)) }
    }
    path.close_path();
    path
}

fn solid(r: u8, g: u8, b: u8) -> Paint {
    Paint::Solid(Rgba8::opaque(r, g, b))
}

fn fill(path: BezPath, paint: Paint, rule: FillRule) -> Node {
    Node::Fill(Fill { transform: Affine::IDENTITY, path, rule, paint })
}

fn solid_fill(path: BezPath, r: u8, g: u8, b: u8) -> Node {
    fill(path, solid(r, g, b), FillRule::NonZero)
}

fn group(children: Vec<Node>) -> Group {
    Group { children, ..Group::default() }
}

fn list(width: f64, height: f64, nodes: Vec<Node>) -> DisplayList {
    DisplayList { size: Size::new(width, height), background: Some(Rgba8::WHITE), nodes }
}

fn star(cx: f64, cy: f64, radius: f64) -> BezPath {
    let points: Vec<(f64, f64)> = (0..5)
        .map(|i| {
            let angle = f64::from(i) * 2.0 * TAU / 5.0 - FRAC_PI_2;
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect();
    poly(&points)
}

fn concat(paths: &[BezPath]) -> BezPath {
    let mut out = BezPath::new();
    for path in paths {
        out.extend(path.iter());
    }
    out
}

fn stops(list: &[(f32, Rgba8)]) -> Vec<GradientStop> {
    list.iter().map(|&(offset, color)| GradientStop { offset, color }).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// fill sides and winding

fn fill_sides_list() -> DisplayList {
    let cw = rect(10.0, 10.0, 40.0, 40.0);
    let ccw = poly(&[(60.0, 10.0), (60.0, 40.0), (90.0, 40.0), (90.0, 10.0)]);
    let bowtie = poly(&[(10.0, 55.0), (45.0, 55.0), (10.0, 90.0), (45.0, 90.0)]);
    let outer = rect(105.0, 5.0, 195.0, 95.0);
    let middle = poly(&[(120.0, 20.0), (120.0, 80.0), (180.0, 80.0), (180.0, 20.0)]);
    let inner = rect(140.0, 40.0, 160.0, 60.0);
    list(
        200.0,
        200.0,
        vec![
            solid_fill(cw, 255, 0, 0),
            solid_fill(ccw, 255, 0, 0),
            solid_fill(bowtie, 255, 0, 0),
            solid_fill(concat(&[outer, middle, inner]), 0, 0, 255),
            solid_fill(star(25.0, 150.0, 22.0), 0, 200, 0),
            fill(star(75.0, 150.0, 22.0), solid(0, 200, 0), FillRule::EvenOdd),
            fill(concat(&[rect(110.0, 110.0, 160.0, 160.0), rect(140.0, 140.0, 190.0, 190.0)]), solid(255, 140, 0), FillRule::EvenOdd),
        ],
    )
}

#[test]
fn fill_sides_and_winding() {
    let shots = shoot("fill_sides", &fill_sides_list(), &ImageStore::new());
    shots.crops("fill_sides", &[("bowtie", 0, 50, 50, 45, 8), ("stars", 0, 120, 100, 60, 5), ("square_edge", 5, 5, 16, 16, 20)]);
    let mut p = Probes::default();
    let red = [255, 0, 0];
    p.both(&shots, "cw square", 25, 25, red, 3);
    p.both(&shots, "ccw square", 75, 25, red, 3);
    p.both(&shots, "between squares", 50, 25, WHITE, 3);
    p.both(&shots, "left edge pixel outside", 9, 25, WHITE, 3);
    p.both(&shots, "left edge pixel inside", 10, 25, red, 3);
    p.both(&shots, "top edge outside", 25, 9, WHITE, 3);
    p.both(&shots, "top edge inside", 25, 10, red, 3);
    p.both(&shots, "bowtie top lobe", 27, 60, red, 3);
    p.both(&shots, "bowtie bottom lobe", 27, 85, red, 3);
    p.both(&shots, "bowtie left notch", 12, 72, WHITE, 3);
    p.both(&shots, "bowtie right notch", 43, 72, WHITE, 3);
    let blue = [0, 0, 255];
    p.both(&shots, "ring left", 110, 50, blue, 3);
    p.both(&shots, "ring right", 190, 50, blue, 3);
    p.both(&shots, "hole", 125, 50, WHITE, 3);
    p.both(&shots, "hole right", 170, 50, WHITE, 3);
    p.both(&shots, "island", 150, 50, blue, 3);
    let green = [0, 200, 0];
    p.both(&shots, "non-zero star centre", 25, 150, green, 3);
    p.both(&shots, "even-odd star centre", 75, 150, WHITE, 3);
    p.both(&shots, "non-zero tip", 25, 135, green, 3);
    p.both(&shots, "even-odd tip", 75, 135, green, 3);
    p.both(&shots, "non-zero between tips", 45, 134, WHITE, 3);
    p.both(&shots, "even-odd between tips", 95, 134, WHITE, 3);
    let orange = [255, 140, 0];
    p.both(&shots, "orange a", 125, 125, orange, 3);
    p.both(&shots, "orange b", 175, 175, orange, 3);
    p.both(&shots, "overlap hole", 150, 150, WHITE, 3);
    p.both(&shots, "outside a", 125, 175, WHITE, 3);
    p.both(&shots, "outside b", 175, 125, WHITE, 3);
    if let Some(r) = &shots.ruffle {
        let share = differing_share(r, &shots.anim, 40);
        p.require(share < 0.01, format!("ruffle and anim-render differ on {:.2}% of the pixels", share * 100.0));
    }
    p.done("fill_sides");
}

// ---------------------------------------------------------------------------------------------------------------
// gradients

fn gradients_list() -> DisplayList {
    let blue = Rgba8::opaque(0, 0, 255);
    let focal = Gradient {
        transform: Affine::translate((50.0, 50.0)) * Affine::scale(40.0 / anim_display::GRADIENT_EXTENT),
        stops: stops(&[(0.0, RED), (1.0, blue)]),
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.9,
    };
    let ramp = |interpolation| Gradient {
        transform: Affine::translate((150.0, 50.0)) * Affine::scale(40.0 / anim_display::GRADIENT_EXTENT),
        stops: stops(&[(0.0, Rgba8::BLACK), (1.0, Rgba8::WHITE)]),
        spread: Spread::Pad,
        interpolation,
        focal: 0.0,
    };
    let repeat = Gradient {
        transform: Affine::translate((50.0, 150.0)) * Affine::rotate(FRAC_PI_2) * Affine::scale(25.0 / anim_display::GRADIENT_EXTENT),
        stops: stops(&[(0.0, RED), (0.2, Rgba8::opaque(0, 255, 0)), (1.0, blue)]),
        spread: Spread::Repeat,
        interpolation: Interpolation::Rgb,
        focal: 0.0,
    };
    let alpha = Gradient {
        transform: Affine::translate((150.0, 150.0)) * Affine::scale(40.0 / anim_display::GRADIENT_EXTENT),
        stops: stops(&[(0.0, Rgba8::new(0, 0, 255, 0)), (1.0, RED)]),
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.0,
    };
    list(
        200.0,
        200.0,
        vec![
            fill(rect(0.0, 0.0, 100.0, 100.0), Paint::Radial(focal), FillRule::NonZero),
            fill(rect(100.0, 0.0, 200.0, 50.0), Paint::Linear(ramp(Interpolation::LinearRgb)), FillRule::NonZero),
            fill(rect(100.0, 50.0, 200.0, 100.0), Paint::Linear(ramp(Interpolation::Rgb)), FillRule::NonZero),
            fill(rect(0.0, 100.0, 100.0, 200.0), Paint::Linear(repeat), FillRule::NonZero),
            solid_fill(rect(100.0, 100.0, 150.0, 200.0), 0, 0, 0),
            solid_fill(rect(150.0, 100.0, 200.0, 200.0), 255, 255, 255),
            fill(rect(100.0, 100.0, 200.0, 200.0), Paint::Linear(alpha), FillRule::NonZero),
        ],
    )
}

#[test]
fn gradients_focal_linear_rgb_repeat_alpha() {
    let shots = shoot("gradients", &gradients_list(), &ImageStore::new());
    shots.crops("gradients", &[("focal", 0, 0, 100, 100, 4), ("alpha", 100, 100, 100, 100, 4)]);
    let mut p = Probes::default();
    p.both(&shots, "focal: just left of the focus", 85, 50, [253, 0, 2], 12);
    p.both(&shots, "focal: centre", 50, 50, [134, 0, 121], 12);
    p.both(&shots, "focal: far left", 11, 50, [5, 0, 250], 12);
    p.both(&shots, "focal: beyond the rim", 95, 50, [0, 0, 255], 8);
    p.both(&shots, "focal: left third", 30, 50, [69, 0, 186], 12);
    p.both(&shots, "focal: near the focus", 70, 50, [203, 0, 52], 12);
    p.both(&shots, "srgb mid", 150, 75, [128, 128, 128], 4);
    p.both(&shots, "srgb quarter", 130, 75, [64, 64, 64], 4);
    p.both(&shots, "srgb three quarters", 170, 75, [191, 191, 191], 4);
    p.both(&shots, "linear mid", 150, 25, [188, 188, 188], 8);
    p.both(&shots, "linear quarter", 130, 25, [137, 137, 137], 8);
    p.both(&shots, "linear three quarters", 170, 25, [225, 225, 225], 8);
    // pixel centres sit half a pixel into the row: y=130 is t=(130.5-125)/50=0.11
    p.both(&shots, "repeat t=0.11 left", 20, 130, [115, 140, 0], 10);
    p.both(&shots, "repeat t=0.11 right", 80, 130, [115, 140, 0], 10);
    p.both(&shots, "repeat t=0.2", 50, 135, [0, 255, 0], 10);
    p.both(&shots, "repeat t=0.6", 50, 155, [0, 128, 128], 10);
    p.both(&shots, "repeat second period", 50, 180, [115, 140, 0], 10);
    p.both(&shots, "repeat before the start", 50, 120, [0, 32, 223], 12);
    p.both(&shots, "alpha over black", 140, 150, [36, 0, 60], 8);
    p.both(&shots, "alpha over white", 160, 150, [195, 96, 156], 8);
    p.done("gradients");
}

// ---------------------------------------------------------------------------------------------------------------
// stroke scale modes

fn stroke_scale_list() -> DisplayList {
    let modes = [StrokeScale::Normal, StrokeScale::Horizontal, StrokeScale::Vertical, StrokeScale::None];
    let style = |scale| StrokeStyle { width: 4.0, cap: Cap::None, join: Join::Miter, miter_limit: 3.0, scale, dashes: vec![] };
    let stroke = |path: BezPath, scale| Node::Stroke(Stroke { transform: Affine::IDENTITY, path, style: style(scale), paint: Paint::Solid(RED) });
    let mut nodes = Vec::new();
    for (i, &scale) in modes.iter().enumerate() {
        let cx = f64::from(u32::try_from(i).unwrap()) * 100.0;
        // row 1: x stretched by 4. A horizontal bar at y=50 and a vertical bar at local x=12.5 (screen 50)
        let mut cross = BezPath::new();
        cross.move_to((3.0, 50.0));
        cross.line_to((22.0, 50.0));
        cross.move_to((12.5, 10.0));
        cross.line_to((12.5, 90.0));
        nodes.push(Node::Group(Group { transform: Affine::translate((cx, 0.0)) * Affine::scale_non_uniform(4.0, 1.0), ..group(vec![stroke(cross, scale)]) }));
        // row 2: y stretched by 4, the bars at screen y=150 and x=50
        let mut cross = BezPath::new();
        cross.move_to((10.0, 12.5));
        cross.line_to((90.0, 12.5));
        cross.move_to((50.0, 3.0));
        cross.line_to((50.0, 22.0));
        nodes.push(Node::Group(Group {
            transform: Affine::translate((cx, 100.0)) * Affine::scale_non_uniform(1.0, 4.0),
            ..group(vec![stroke(cross, scale)])
        }));
    }
    list(400.0, 200.0, nodes)
}

fn is_red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn run_x(image: &Rendered, y: usize, x0: usize, x1: usize) -> usize {
    (x0..x1).filter(|&x| is_red(image.at(x, y))).count()
}

fn run_y(image: &Rendered, x: usize, y0: usize, y1: usize) -> usize {
    (y0..y1).filter(|&y| is_red(image.at(x, y))).count()
}

#[test]
fn stroke_scale_modes_under_non_uniform_scale() {
    let shots = shoot("stroke_scale", &stroke_scale_list(), &ImageStore::new());
    shots.crops("stroke_scale", &[("row1_normal", 30, 20, 40, 40, 10), ("row1_horizontal", 130, 20, 40, 40, 10)]);
    let mut p = Probes::default();
    // (vertical bar width in row 1, horizontal bar height in row 1, vertical bar width in row 2, horizontal bar height in row 2)
    let expected = [(16, 4, 4, 16), (4, 4, 4, 16), (16, 4, 4, 4), (4, 4, 4, 4)];
    let names = ["normal", "horizontal", "vertical", "none"];
    // Ruffle does not implement the NoHScale / NoVScale bits (it draws every mode like Normal, even for a shape placed
    // directly under the scale), so the SWF side is checked by reading the bits back; the pixels come from anim-render.
    {
        let (who, image) = ("anim", &shots.anim);
        for (i, (&(vw, hh, vw2, hh2), name)) in expected.iter().zip(names).enumerate() {
            let cx = i * 100;
            // row 1: the vertical bar is crossed by the scan line y=30, the horizontal bar by the column cx+20
            let got_vw = run_x(image, 30, cx, cx + 100);
            let got_hh = run_y(image, cx + 20, 0, 100);
            // row 2: the horizontal bar is crossed by the column cx+20, the vertical bar by the scan line y=130... which
            // is above the vertical bar (it spans y 112..188), so it crosses it
            let got_vw2 = run_x(image, 130, cx, cx + 100);
            let got_hh2 = run_y(image, cx + 20, 100, 200);
            for (what, got, want) in [("row 1 vertical bar width", got_vw, vw), ("row 1 horizontal bar height", got_hh, hh), ("row 2 vertical bar width", got_vw2, vw2), ("row 2 horizontal bar height", got_hh2, hh2)] {
                p.require(got.abs_diff(want) <= 1, format!("[{who}] {name}: {what} is {got}, expected {want}"));
            }
        }
    }
    // the bits: NoHScale keeps the thickness from following horizontal scaling, NoVScale vertical scaling
    let list = stroke_scale_list();
    let out = export_display_lists([&list], &ImageStore::new(), export_options(&list)).unwrap();
    let decompressed = swf::decompress_swf(&out.swf[..]).unwrap();
    let parsed = swf::parse_swf(&decompressed).unwrap();
    let shapes: Vec<_> = parsed.tags.iter().filter_map(|t| if let swf::Tag::DefineShape(s) = t { Some(s) } else { None }).collect();
    // two shapes (row 1 and row 2) per mode, in order
    let allow = [(true, true), (false, true), (true, false), (false, false)];
    for (i, (&(ax, ay), name)) in allow.iter().zip(names).enumerate() {
        for shape in shapes.iter().skip(i * 2).take(2) {
            let line = &shape.styles.line_styles[0];
            p.require((line.allow_scale_x(), line.allow_scale_y()) == (ax, ay), format!("[bits] {name}: allow_scale (x, y) is ({}, {}), expected ({ax}, {ay})", line.allow_scale_x(), line.allow_scale_y()));
        }
    }
    p.done("stroke_scale");
}

// ---------------------------------------------------------------------------------------------------------------
// nested colour transforms

fn ct(mul: [f32; 4], add: [f32; 4]) -> ColorTransform {
    ColorTransform { mul, add }
}

fn nested_cx_list() -> DisplayList {
    // cell 1
    let inner = Group { color: ct([0.5, 1.0, 1.0, 1.0], [0.0, 51.0 / 255.0, 0.0, 0.0]), ..group(vec![solid_fill(rect(0.0, 0.0, 40.0, 40.0), 255, 0, 0)]) };
    let outer = Group {
        transform: Affine::new([1.0, 0.0, 0.5, 1.0, 20.0, 20.0]),
        color: ct([1.0, 1.0, 1.0, 0.5], [0.0; 4]),
        ..group(vec![Node::Group(inner)])
    };
    // cell 2
    let inner2 = Group { color: ct([1.0, 1.0, 1.0, 0.5], [0.0; 4]), ..group(vec![solid_fill(rect(-30.0, -30.0, 30.0, 30.0), 0, 0, 255)]) };
    let middle2 = Group { transform: Affine::rotate(FRAC_PI_4), color: ct([1.0, 1.0, 1.0, 0.5], [0.0, 0.0, 0.0, 0.5]), ..group(vec![Node::Group(inner2)]) };
    let outer2 = Group { transform: Affine::translate((150.0, 50.0)), color: ct([1.0, 1.0, 1.0, 0.5], [0.0; 4]), ..group(vec![Node::Group(middle2)]) };
    // cell 3
    let inner3 = Group { color: ct([1.0, 3.0, 1.0, 1.0], [0.0, 0.0, 0.5, 0.0]), ..group(vec![solid_fill(rect(20.0, 120.0, 80.0, 180.0), 0, 128, 0)]) };
    let outer3 = Group { color: ct([0.5, 0.5, 0.5, 1.0], [0.0; 4]), ..group(vec![Node::Group(inner3)]) };
    // cell 4
    let reveal = Group {
        color: ct([1.0; 4], [0.0, 0.0, 0.0, 1.0]),
        ..group(vec![fill(rect(120.0, 120.0, 180.0, 180.0), Paint::Solid(Rgba8::new(255, 0, 0, 0)), FillRule::NonZero)])
    };
    list(200.0, 200.0, vec![Node::Group(outer), Node::Group(outer2), Node::Group(outer3), Node::Group(reveal)])
}

#[test]
fn nested_colour_transforms_concatenate() {
    let shots = shoot("nested_cx", &nested_cx_list(), &ImageStore::new());
    shots.crops("nested_cx", &[("skew", 10, 10, 80, 60, 6), ("diamond", 110, 10, 80, 80, 5)]);
    let mut p = Probes::default();
    let c1 = [191, 153, 128];
    p.both(&shots, "cell 1 centre", 50, 40, c1, 4);
    p.both(&shots, "cell 1 top left", 30, 25, c1, 4);
    p.both(&shots, "cell 1 bottom right", 75, 55, c1, 4);
    p.both(&shots, "cell 1 outside left", 22, 45, WHITE, 3);
    p.both(&shots, "cell 1 outside right", 78, 55, WHITE, 3);
    p.both(&shots, "cell 1 outside corner", 10, 10, WHITE, 3);
    let c2 = [159, 159, 255];
    p.both(&shots, "cell 2 centre", 150, 50, c2, 4);
    p.both(&shots, "cell 2 inside diamond", 190, 50, c2, 4);
    p.both(&shots, "cell 2 inside diamond b", 175, 65, c2, 4);
    p.both(&shots, "cell 2 outside diamond", 180, 70, WHITE, 3);
    // anim-render clamps after every level (g=128), Flash concatenates the transforms first (g=192): libs divergence
    p.check("anim", "cell 3 (anim-render clamps between levels)", &shots.anim, 50, 150, [0, 128, 64], 5);
    // and anim-render drops fills with alpha 0 even under an alpha offset
    p.check("anim", "cell 4 (anim-render drops alpha 0 fills)", &shots.anim, 150, 150, WHITE, 4);
    if let Some(r) = &shots.ruffle {
        p.check("ruffle", "cell 3 concatenated without clamping", r, 50, 150, [0, 192, 64], 5);
        p.check("ruffle", "cell 4 transparent fill revealed", r, 150, 150, [255, 0, 0], 4);
    }
    p.done("nested_cx");
}

// ---------------------------------------------------------------------------------------------------------------
// masks

fn masks_list() -> DisplayList {
    // cell A
    let ring = fill(concat(&[rect(10.0, 10.0, 90.0, 90.0), rect(30.0, 30.0, 70.0, 70.0)]), solid(0, 0, 0), FillRule::EvenOdd);
    let a = Group { mask: Some(vec![ring]), ..group(vec![solid_fill(rect(0.0, 0.0, 100.0, 100.0), 0, 200, 0)]) };
    // cell B
    let faint = fill(rect(110.0, 10.0, 190.0, 90.0), Paint::Solid(Rgba8::new(0, 0, 0, 77)), FillRule::NonZero);
    let b = Group { mask: Some(vec![faint]), ..group(vec![solid_fill(rect(100.0, 0.0, 200.0, 100.0), 0, 0, 255)]) };
    // cell C
    let inner = Group {
        mask: Some(vec![solid_fill(rect(0.0, 100.0, 50.0, 200.0), 0, 0, 0)]),
        ..group(vec![solid_fill(rect(0.0, 100.0, 100.0, 200.0), 255, 255, 0)])
    };
    let c = Group {
        mask: Some(vec![solid_fill(Circle::new((50.0, 150.0), 40.0).to_path(0.05), 0, 0, 0)]),
        ..group(vec![Node::Group(inner), solid_fill(rect(60.0, 140.0, 80.0, 160.0), 255, 0, 255)])
    };
    // cell D
    let d = Group {
        mask: Some(vec![solid_fill(rect(110.0, 110.0, 130.0, 130.0), 0, 0, 0), solid_fill(rect(150.0, 150.0, 170.0, 170.0), 0, 0, 0)]),
        ..group(vec![solid_fill(rect(100.0, 100.0, 200.0, 200.0), 0, 255, 255)])
    };
    list(
        200.0,
        200.0,
        vec![Node::Group(a), Node::Group(b), Node::Group(c), Node::Group(d), solid_fill(rect(172.0, 112.0, 192.0, 132.0), 255, 0, 0)],
    )
}

#[test]
fn masks_holes_alpha_nesting_and_siblings() {
    let shots = shoot("masks", &masks_list(), &ImageStore::new());
    shots.crops("masks", &[("nested", 0, 100, 100, 100, 5), ("union", 100, 100, 100, 100, 5)]);
    let mut p = Probes::default();
    let green = [0, 200, 0];
    p.both(&shots, "ring", 20, 50, green, 3);
    p.both(&shots, "ring corner", 80, 80, green, 3);
    p.both(&shots, "hole", 50, 50, WHITE, 3);
    p.both(&shots, "outside", 5, 5, WHITE, 3);
    p.both(&shots, "outside right", 95, 50, WHITE, 3);
    // A Flash mask ignores the alpha of what it is painted with. anim-render weights by it (178,178,255), against its
    // own doc ("drawn only where the mask's alpha is non-zero"): a divergence in anicraft-libs, not in the exporter.
    p.check("anim", "translucent mask (anim-render weights by alpha)", &shots.anim, 150, 50, [178, 178, 255], 4);
    if let Some(r) = &shots.ruffle {
        p.check("ruffle", "translucent mask is opaque", r, 150, 50, [0, 0, 255], 4);
    }
    p.both(&shots, "translucent mask outside", 105, 50, WHITE, 3);
    p.both(&shots, "nested: yellow", 30, 150, [255, 255, 0], 3);
    p.both(&shots, "nested: inner edge inside", 49, 150, [255, 255, 0], 3);
    p.both(&shots, "nested: inner edge outside", 50, 150, WHITE, 3);
    p.both(&shots, "nested: magenta not subject to the inner mask", 70, 150, [255, 0, 255], 3);
    p.both(&shots, "nested: magenta lower", 75, 155, [255, 0, 255], 3);
    p.both(&shots, "nested: circle, right half", 70, 120, WHITE, 3);
    p.both(&shots, "nested: outside the circle", 10, 110, WHITE, 3);
    p.both(&shots, "union mask a", 120, 120, [0, 255, 255], 3);
    p.both(&shots, "union mask b", 160, 160, [0, 255, 255], 3);
    p.both(&shots, "union mask gap", 140, 140, WHITE, 3);
    p.both(&shots, "union mask outside", 120, 160, WHITE, 3);
    p.both(&shots, "sibling above the mask", 180, 120, [255, 0, 0], 3);
    p.both(&shots, "no cyan beside the sibling", 160, 120, WHITE, 3);
    p.done("masks");
}

// ---------------------------------------------------------------------------------------------------------------
// filters and blends

fn filters_blend_list() -> DisplayList {
    let blur = Group {
        filters: vec![Filter::Blur { x: 8.0, y: 0.0, quality: 3 }],
        ..group(vec![solid_fill(rect(30.0, 30.0, 70.0, 70.0), 255, 0, 0)])
    };
    let shadow = Group {
        filters: vec![Filter::DropShadow {
            color: Rgba8::BLACK,
            angle_deg: 45.0,
            distance: 10.0,
            blur_x: 0.0,
            blur_y: 0.0,
            strength: 1.0,
            inner: false,
            knockout: true,
            hide_object: false,
        }],
        ..group(vec![solid_fill(rect(130.0, 30.0, 170.0, 70.0), 0, 0, 255)])
    };
    let multiply = Group {
        blend: BlendMode::Multiply,
        ..group(vec![solid_fill(rect(10.0, 110.0, 60.0, 160.0), 0, 255, 255), solid_fill(rect(40.0, 140.0, 90.0, 190.0), 255, 0, 255)])
    };
    let difference = Group {
        blend: BlendMode::Difference,
        color: ct([1.0, 1.0, 1.0, 0.5], [0.0; 4]),
        ..group(vec![solid_fill(rect(120.0, 120.0, 180.0, 180.0), 255, 255, 255)])
    };
    list(
        200.0,
        200.0,
        vec![
            Node::Group(blur),
            Node::Group(shadow),
            solid_fill(rect(0.0, 100.0, 100.0, 200.0), 255, 255, 0),
            Node::Group(multiply),
            solid_fill(rect(100.0, 100.0, 200.0, 200.0), 200, 50, 50),
            Node::Group(difference),
        ],
    )
}

#[test]
fn filters_knockout_and_blend_flattening() {
    let shots = shoot("filters_blend", &filters_blend_list(), &ImageStore::new());
    shots.crops("filters_blend", &[("blur", 10, 20, 80, 60, 6), ("shadow", 110, 20, 80, 70, 6), ("blend", 0, 100, 200, 100, 4)]);
    let mut p = Probes::default();
    p.both(&shots, "blur: crisp top edge outside", 50, 29, WHITE, 6);
    p.both(&shots, "blur: crisp top edge inside", 50, 31, [255, 0, 0], 40);
    p.both(&shots, "blur: no vertical leak", 50, 26, WHITE, 6);
    p.both(&shots, "blur: far side of the edge", 20, 50, [255, 200, 200], 60);
    p.both(&shots, "blur: inside", 40, 50, [255, 0, 0], 30);
    for (image, who) in [(Some(&shots.anim), "anim"), (shots.ruffle.as_ref(), "ruffle")] {
        let Some(image) = image else { continue };
        for d in 1..=6_usize {
            let sum = u32::from(image.at(30 - d, 50)[1]) + u32::from(image.at(29 + d, 50)[1]);
            p.require(sum.abs_diff(255) <= 40, format!("[{who}] blur is not symmetric about x=30 at d={d}: {sum}"));
        }
    }
    p.both(&shots, "knockout: object removed", 150, 50, WHITE, 3);
    p.both(&shots, "knockout: left", 125, 50, WHITE, 3);
    p.both(&shots, "knockout: shadow only", 172, 72, [0, 0, 0], 20);
    p.both(&shots, "knockout: shadow below", 150, 75, [0, 0, 0], 20);
    p.both(&shots, "knockout: below the shadow", 150, 80, WHITE, 6);
    p.both(&shots, "multiply: cyan on yellow", 20, 120, [0, 255, 0], 4);
    p.both(&shots, "multiply: magenta on yellow", 80, 180, [255, 0, 0], 4);
    p.both(&shots, "multiply: overlap is the top layer only", 50, 150, [255, 0, 0], 4);
    p.both(&shots, "multiply: untouched yellow", 95, 195, [255, 255, 0], 4);
    p.both(&shots, "difference with alpha", 150, 150, [128, 128, 128], 6);
    p.both(&shots, "difference: outside", 110, 110, [200, 50, 50], 3);
    p.done("filters_blend");
}

// ---------------------------------------------------------------------------------------------------------------
// joins, caps, dashes

fn joins_list() -> DisplayList {
    let dark = Paint::Solid(Rgba8::opaque(30, 30, 30));
    let stroke = |path: BezPath, width: f64, cap: Cap, join: Join, miter_limit: f64, dashes: Vec<f64>| {
        Node::Stroke(Stroke {
            transform: Affine::IDENTITY,
            path,
            style: StrokeStyle { width, cap, join, miter_limit, scale: StrokeScale::Normal, dashes },
            paint: dark.clone(),
        })
    };
    let open = |points: &[(f64, f64)]| {
        let mut path = BezPath::new();
        for (i, &(x, y)) in points.iter().enumerate() {
            if i == 0 { path.move_to((x, y)) } else { path.line_to((x, y)) }
        }
        path
    };
    let half = 30.0_f64.to_radians().tan() * 40.0;
    let half2 = 10.0_f64.to_radians().tan() * 70.0;
    let nodes = vec![
        stroke(open(&[(25.0 - half, 60.0), (25.0, 20.0), (25.0 + half, 60.0)]), 10.0, Cap::None, Join::Miter, 3.0, vec![]),
        stroke(open(&[(75.0 - half2, 85.0), (75.0, 15.0), (75.0 + half2, 85.0)]), 10.0, Cap::None, Join::Miter, 3.0, vec![]),
        stroke(open(&[(115.0, 20.0), (155.0, 20.0)]), 12.0, Cap::None, Join::Round, 3.0, vec![]),
        stroke(open(&[(115.0, 50.0), (155.0, 50.0)]), 12.0, Cap::Round, Join::Round, 3.0, vec![]),
        stroke(open(&[(115.0, 80.0), (155.0, 80.0)]), 12.0, Cap::Square, Join::Round, 3.0, vec![]),
        stroke(poly(&[(10.0, 110.0), (90.0, 110.0), (90.0, 190.0), (10.0, 190.0)]), 4.0, Cap::None, Join::Miter, 3.0, vec![10.0, 6.0]),
        stroke(open(&[(110.0, 125.0), (180.0, 125.0), (110.0, 127.0)]), 10.0, Cap::None, Join::Round, 3.0, vec![]),
        stroke(open(&[(110.0, 175.0), (180.0, 175.0), (110.0, 177.0)]), 10.0, Cap::None, Join::Miter, 4.0, vec![]),
    ];
    list(200.0, 200.0, nodes)
}

#[test]
fn joins_caps_and_dashes() {
    let shots = shoot("joins_caps_dashes", &joins_list(), &ImageStore::new());
    shots.crops(
        "joins_caps_dashes",
        &[("chevrons", 0, 0, 100, 100, 6), ("caps", 100, 0, 100, 100, 6), ("dashes_top_left", 0, 100, 50, 50, 12), ("dashes_corner", 60, 100, 40, 40, 12), ("hairpins", 140, 100, 60, 100, 6)],
    );
    let mut p = Probes::default();
    p.both(&shots, "miter tip kept", 25, 12, DARK, 40);
    p.both(&shots, "miter tip end", 25, 8, WHITE, 20);
    // Beyond the miter limit anim-render bevels (SVG); Flash and Ruffle clip the miter at limit * half width (15 px)
    // from the vertex instead, so the spike still reaches (75,1).
    p.check("anim", "bevel flat top outside", &shots.anim, 75, 13, WHITE, 20);
    if let Some(r) = &shots.ruffle {
        p.check("ruffle", "clipped miter spike", r, 75, 5, DARK, 60);
    }
    p.both(&shots, "bevel apex inside", 75, 16, DARK, 40);
    p.both(&shots, "no cap before", 114, 20, WHITE, 20);
    p.both(&shots, "no cap at", 116, 20, DARK, 40);
    p.both(&shots, "round cap centre", 110, 50, DARK, 40);
    p.both(&shots, "round cap beyond", 108, 50, WHITE, 20);
    p.both(&shots, "round cap corner is cut", 110, 45, [200, 200, 200], 56);
    p.both(&shots, "square cap", 110, 80, DARK, 40);
    p.both(&shots, "square cap corner", 110, 75, DARK, 40);
    p.both(&shots, "square cap beyond", 108, 80, WHITE, 20);
    p.both(&shots, "dash 1 on", 15, 110, DARK, 50);
    p.both(&shots, "dash gap", 23, 110, WHITE, 20);
    p.both(&shots, "dash 2 on", 31, 110, DARK, 50);
    p.both(&shots, "dash 2 gap", 39, 110, WHITE, 20);
    p.both(&shots, "corner dash starts", 90, 115, DARK, 50);
    p.both(&shots, "corner dash gap", 90, 123, WHITE, 20);
    p.both(&shots, "corner dash 2", 90, 130, DARK, 50);
    p.both(&shots, "corner dash has a butt end", 91, 109, WHITE, 20);
    p.both(&shots, "left edge dash", 10, 185, DARK, 50);
    p.both(&shots, "left edge gap", 10, 177, WHITE, 20);
    p.both(&shots, "closing gap", 10, 113, WHITE, 20);
    p.both(&shots, "last dash before the closing gap", 10, 120, DARK, 50);
    p.both(&shots, "round hairpin", 183, 126, DARK, 50);
    p.both(&shots, "round hairpin beyond", 186, 126, WHITE, 30);
    p.both(&shots, "bevelled hairpin", 178, 176, DARK, 50);
    p.both(&shots, "bevelled hairpin beyond", 182, 176, WHITE, 30);
    p.done("joins_caps_dashes");
}

// ---------------------------------------------------------------------------------------------------------------
// frame differ

fn multiframe_lists() -> Vec<DisplayList> {
    let cyan = || solid_fill(rect(0.0, 0.0, 40.0, 40.0), 0, 255, 255);
    let yellow = || solid_fill(rect(0.0, 0.0, 100.0, 100.0), 255, 255, 0);
    let red = || solid_fill(rect(70.0, 70.0, 90.0, 90.0), 255, 0, 0);
    let g = |x: f64, blend, color: ColorTransform, filters: Vec<Filter>, mask: Option<Vec<Node>>| {
        Node::Group(Group { transform: Affine::translate((x, 20.0)), blend, color, filters, mask, ..group(vec![cyan()]) })
    };
    let frame = |nodes| list(100.0, 100.0, nodes);
    vec![
        frame(vec![
            yellow(),
            g(20.0, BlendMode::Multiply, ct([1.0, 1.0, 1.0, 0.5], [0.0; 4]), vec![Filter::Blur { x: 4.0, y: 4.0, quality: 1 }], None),
            red(),
        ]),
        frame(vec![yellow(), g(20.0, BlendMode::Normal, ColorTransform::IDENTITY, vec![], None), red()]),
        frame(vec![yellow(), g(50.0, BlendMode::Subtract, ColorTransform::IDENTITY, vec![], None), red()]),
        frame(vec![yellow(), g(20.0, BlendMode::Normal, ColorTransform::IDENTITY, vec![], None), solid_fill(rect(10.0, 70.0, 30.0, 90.0), 0, 200, 0)]),
        frame(vec![
            yellow(),
            g(20.0, BlendMode::Normal, ColorTransform::IDENTITY, vec![], Some(vec![solid_fill(rect(0.0, 0.0, 20.0, 40.0), 0, 0, 0)])),
            solid_fill(rect(10.0, 70.0, 30.0, 90.0), 0, 200, 0),
        ]),
        frame(vec![]),
    ]
}

#[test]
fn frame_differ_resets_and_replaces() {
    let lists = multiframe_lists();
    let images = ImageStore::new();
    let mut options = export_options(lists.first().unwrap());
    options.loop_playback = false;
    let out = export_display_lists(lists.iter(), &images, options).unwrap();
    let ruffle = common::render_swf_frames("multiframe_diff", &out.swf, lists.len());
    let anim: Vec<Rendered> = lists.iter().map(|l| anim_render(l, &images)).collect();
    for (i, image) in anim.iter().enumerate() {
        save(&format!("multiframe_diff.anim.f{i}"), image);
    }
    if let Some(frames) = &ruffle {
        for (i, image) in frames.iter().enumerate() {
            save(&format!("multiframe_diff.ruffle.f{i}"), image);
        }
    }
    let mut p = Probes::default();
    let yellow = [255, 255, 0];
    let cyan = [0, 255, 255];
    let mut at = |frame: usize, label: &str, x: usize, y: usize, want: [u8; 3], tol: i32| {
        p.check("anim", &format!("f{frame} {label}"), &anim[frame], x, y, want, tol);
        if let Some(frames) = &ruffle {
            p.check("ruffle", &format!("f{frame} {label}"), &frames[frame], x, y, want, tol);
        }
    };
    at(0, "multiplied interior", 40, 40, [128, 255, 0], 10);
    at(0, "outside", 10, 10, yellow, 3);
    at(0, "red", 80, 80, [255, 0, 0], 3);
    at(1, "opaque cyan", 40, 40, cyan, 3);
    at(1, "crisp left edge outside", 19, 40, yellow, 3);
    at(1, "crisp left edge inside", 20, 40, cyan, 3);
    at(1, "crisp right edge inside", 59, 40, cyan, 3);
    at(1, "crisp right edge outside", 60, 40, yellow, 3);
    at(2, "subtracted", 70, 40, [255, 0, 0], 4);
    at(2, "old place is yellow", 30, 40, yellow, 3);
    at(3, "red removed", 80, 80, yellow, 3);
    at(3, "green added", 20, 80, [0, 200, 0], 3);
    at(3, "cyan back", 40, 40, cyan, 3);
    at(4, "masked left half", 30, 40, cyan, 3);
    at(4, "masked right half", 50, 40, yellow, 3);
    at(4, "green stays", 20, 80, [0, 200, 0], 3);
    at(5, "empty", 50, 50, WHITE, 3);
    at(5, "empty where cyan was", 30, 40, WHITE, 3);
    p.done("multiframe_diff");
}

// ---------------------------------------------------------------------------------------------------------------
// buttons, timelines, scripts and sounds (movie level)

fn square_shape(movie: &mut Movie, x0: f64, y0: f64, x1: f64, y1: f64, color: Rgba8) -> CharacterId {
    let mut b = ShapeBuilder::new();
    let fill = b.add_fill(FillStyle::Solid(color)).unwrap();
    b.add_path(&Rect::new(x0, y0, x1, y1).to_path(0.1), PathStyle { fill1: fill, ..PathStyle::default() });
    movie.define_shape(&b.finish()).unwrap()
}

fn movie_settings(width: f64, height: f64) -> MovieSettings {
    MovieSettings { width, height, compression: Compression::None, ..MovieSettings::default() }
}

fn button_swf() -> Vec<u8> {
    let mut movie = Movie::new(movie_settings(100.0, 100.0)).unwrap();
    let red = square_shape(&mut movie, 0.0, 0.0, 20.0, 20.0, Rgba8::opaque(255, 0, 0));
    let cyan = square_shape(&mut movie, 0.0, 0.0, 10.0, 10.0, Rgba8::opaque(0, 255, 255));
    let green = square_shape(&mut movie, 0.0, 0.0, 10.0, 10.0, Rgba8::opaque(0, 255, 0));
    let blue = square_shape(&mut movie, 10.0, 0.0, 20.0, 10.0, Rgba8::opaque(0, 0, 255));
    let yellow = square_shape(&mut movie, 0.0, 0.0, 40.0, 40.0, Rgba8::opaque(255, 255, 0));
    let mut multiply = ButtonRecord::new(ButtonStates::UP, cyan, 2);
    multiply.matrix = Matrix::from_affine(Affine::translate((5.0, 5.0)));
    multiply.blend = Some(BlendMode::Multiply);
    let button = movie
        .define_button(&ButtonDef {
            track_as_menu: false,
            records: vec![
                ButtonRecord::new(ButtonStates { up: true, over: true, down: false, hit: false }, red, 1),
                multiply,
                ButtonRecord::new(ButtonStates::OVER, green, 3),
                ButtonRecord::new(ButtonStates::DOWN, blue, 4),
                ButtonRecord::new(ButtonStates::HIT, yellow, 5),
            ],
            actions: vec![],
        })
        .unwrap();
    movie.add_frame(&Frame::new(vec![FrameItem::Place(Placement::new(1, button, Matrix::from_affine(Affine::translate((10.0, 10.0)) * Affine::scale(2.0))))])).unwrap();
    movie.finish().unwrap()
}

#[test]
fn buttons_show_only_the_up_state() {
    let swf = button_swf();
    let Some(r) = common::render_swf("button_up", &swf) else { return };
    save("button_up.ruffle", &r);
    let mut p = Probes::default();
    p.check("ruffle", "over square is not drawn", &r, 15, 15, [255, 0, 0], 3);
    p.check("ruffle", "down square is not drawn", &r, 45, 15, [255, 0, 0], 3);
    p.check("ruffle", "multiply record (black on red)", &r, 30, 30, [0, 0, 0], 6);
    p.check("ruffle", "up record", &r, 45, 45, [255, 0, 0], 3);
    p.check("ruffle", "hit area is invisible", &r, 70, 70, WHITE, 3);
    p.check("ruffle", "outside", &r, 55, 55, WHITE, 3);
    p.done("button_up");
}

/// `DoAction` bytecode: goto frame 1 (zero based), then play.
const GOTO_FRAME_1_PLAY: [u8; 7] = [0x81, 0x02, 0x00, 0x01, 0x00, 0x06, 0x00];

fn timeline_swf(script_on_frame_3: &[u8], with_sound_and_sprite: bool) -> Vec<u8> {
    let mut movie = Movie::new(movie_settings(120.0, 60.0)).unwrap();
    let red = square_shape(&mut movie, 0.0, 0.0, 10.0, 10.0, Rgba8::opaque(255, 0, 0));
    let blue = square_shape(&mut movie, 0.0, 0.0, 10.0, 10.0, Rgba8::opaque(0, 0, 255));
    let at = |x: f64, y: f64| Matrix::from_affine(Affine::translate((x, y)));
    let mov = |x: f64, y: f64| FrameItem::Place(Placement { depth: 1, is_move: true, matrix: Some(at(x, y)), ..Placement::default() });
    let sprite = movie
        .define_sprite(&[
            Frame::new(vec![FrameItem::Place(Placement::new(1, blue, at(10.0, 40.0)))]),
            Frame::new(vec![mov(30.0, 40.0), FrameItem::DoAction(ACTION_STOP.to_vec())]),
            Frame::new(vec![mov(50.0, 40.0)]),
        ])
        .unwrap();
    let mut first = vec![FrameItem::Place(Placement::new(1, red, at(10.0, 10.0)))];
    if with_sound_and_sprite {
        let samples: Vec<i16> = (0..4410).map(|i| i16::try_from(i % 100).unwrap() * 100).collect();
        let sound = movie.define_sound_pcm(&samples, 1, 44_100).unwrap();
        first.push(FrameItem::StartSound { sound, info: SoundInfo::default() });
        first.push(FrameItem::Place(Placement::new(2, sprite, Matrix::IDENTITY)));
    }
    movie.add_frame(&Frame::new(first)).unwrap();
    for i in 1..6_u32 {
        let mut items = vec![mov(10.0 + 15.0 * f64::from(i), 10.0)];
        if i == 3 {
            items.push(FrameItem::DoAction(script_on_frame_3.to_vec()));
        }
        movie.add_frame(&Frame::new(items)).unwrap();
    }
    movie.finish().unwrap()
}

fn first_x(image: &Rendered, y: usize, want: fn([u8; 3]) -> bool) -> Option<usize> {
    (0..image.width).find(|&x| want(image.at(x, y)))
}

fn is_blue(p: [u8; 3]) -> bool {
    p[2] > 200 && p[0] < 60 && p[1] < 60
}

#[test]
fn stop_holds_the_main_timeline_and_a_clip_stops_on_its_own() {
    let swf = timeline_swf(&ACTION_STOP, true);
    let Some(frames) = common::render_swf_frames("timeline_stop", &swf, 6) else { return };
    for (i, f) in frames.iter().enumerate() {
        save(&format!("timeline_stop.ruffle.f{i}"), f);
    }
    let red: Vec<Option<usize>> = frames.iter().map(|f| first_x(f, 15, is_red_px)).collect();
    let blue: Vec<Option<usize>> = frames.iter().map(|f| first_x(f, 45, is_blue)).collect();
    assert_eq!(red, [10, 25, 40, 55, 55, 55].map(Some), "main timeline");
    assert_eq!(blue, [10, 30, 30, 30, 30, 30].map(Some), "clip timeline");
}

fn is_red_px(p: [u8; 3]) -> bool {
    is_red(p)
}

#[test]
fn goto_and_play_jump_back() {
    let swf = timeline_swf(&GOTO_FRAME_1_PLAY, false);
    let Some(frames) = common::render_swf_frames("timeline_goto", &swf, 8) else { return };
    let red: Vec<Option<usize>> = frames.iter().map(|f| first_x(f, 15, is_red_px)).collect();
    // the script on frame 3 runs before the frame is drawn, so frame 3 itself is never shown
    assert_eq!(red, [10, 25, 40, 25, 40, 25, 40, 25].map(Some), "goto frame 1 from frame 3");
}

#[test]
fn sounds_and_buttons_parse_back() {
    use swf::{Tag, parse_swf};
    let buf = swf::decompress_swf(&timeline_swf(&ACTION_STOP, true)[..]).unwrap();
    let movie = parse_swf(&buf).unwrap();
    let starts = movie.tags.iter().filter(|t| matches!(t, Tag::StartSound(_))).count();
    let defs = movie.tags.iter().filter(|t| matches!(t, Tag::DefineSound(_))).count();
    assert_eq!((starts, defs), (1, 1));
    let actions = movie.tags.iter().filter(|t| matches!(t, Tag::DoAction(_))).count();
    assert_eq!(actions, 1, "one stop on the main timeline");

    let buf = swf::decompress_swf(&button_swf()[..]).unwrap();
    let movie = parse_swf(&buf).unwrap();
    let button = movie.tags.iter().find_map(|t| if let Tag::DefineButton2(b) = t { Some(b) } else { None }).unwrap();
    assert_eq!(button.records.len(), 5);
    // up|over, up, over, down, hit
    let states: Vec<_> = button.records.iter().map(|r| r.states.bits()).collect();
    assert_eq!(states, [0b0011, 0b0001, 0b0010, 0b0100, 0b1000]);
    let blend = button.records.get(1).map(|r| r.blend_mode);
    assert_eq!(blend, Some(swf::BlendMode::Multiply));
}

/// What Ruffle does with the `NoHScale` / `NoVScale` bits of `LINESTYLE2`, with the shape placed directly under a
/// (4,1) scale (the SWF rule is that `NoHScale` keeps the thickness from following horizontal scaling).
#[test]
fn player_follows_the_stroke_scale_bits() {
    use anicraft_swf::style::{LinePaint, LineStyle};
    let mut movie = Movie::new(movie_settings(100.0, 100.0)).unwrap();
    let mut ids = Vec::new();
    for (no_h, no_v) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut b = ShapeBuilder::new();
        let line = b
            .add_line(LineStyle {
                width: 80,
                paint: LinePaint::Solid(RED),
                start_cap: Cap::None,
                end_cap: Cap::None,
                join: Join::Miter,
                miter_limit: 3.0,
                no_h_scale: no_h,
                no_v_scale: no_v,
                pixel_hinting: false,
                no_close: false,
            })
            .unwrap();
        let mut path = BezPath::new();
        path.move_to((5.0, 10.0));
        path.line_to((5.0, 40.0));
        b.add_path(&path, PathStyle { line, ..PathStyle::default() });
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((20.0, 0.0));
        b.add_path(&path, PathStyle { line, ..PathStyle::default() });
        ids.push(movie.define_shape(&b.finish()).unwrap());
    }
    let items = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let m = Affine::translate((0.0, 25.0 * i as f64)) * Affine::scale_non_uniform(4.0, 1.0);
            FrameItem::Place(Placement::new(u16::try_from(i + 1).unwrap(), id, Matrix::from_affine(m)))
        })
        .collect();
    movie.add_frame(&Frame::new(items)).unwrap();
    let swf = movie.finish().unwrap();
    // with 80 twips = 4 px lines, a vertical bar at local x=5 -> screen x=20; the horizontal bar of each shape at y = 25*i
    let Some(r) = common::render_swf("stroke_bits_direct", &swf) else { return };
    save("stroke_bits_direct.ruffle", &r);
    let widths: Vec<usize> = (0..4).map(|i| run_x(&r, 25 * i + 20, 0, 100)).collect();
    // The spec rule would give 16, 4, 16, 4 for (no_h, no_v) = (0,0), (1,0), (0,1), (1,1). Ruffle ignores the bits and
    // draws 16 everywhere; this test only records what it does, it cannot serve as an oracle for the bits.
    assert!(widths.iter().all(|w| (4..=16).contains(w)), "vertical bar widths under a horizontal stretch: {widths:?}");
}

/// A fill with alpha 0 under a colour transform that adds alpha: does a player show it?
#[test]
fn player_reveals_transparent_fills_with_an_alpha_offset() {
    use anicraft_swf::types::CxForm;
    let mut movie = Movie::new(movie_settings(100.0, 100.0)).unwrap();
    let hidden = square_shape(&mut movie, 0.0, 0.0, 40.0, 40.0, Rgba8::new(255, 0, 0, 0));
    let mut place = Placement::new(1, hidden, Matrix::from_affine(Affine::translate((20.0, 20.0))));
    place.color = Some(CxForm { mult: [256; 4], add: [0, 0, 0, 255] });
    movie.add_frame(&Frame::new(vec![FrameItem::Place(place)])).unwrap();
    let swf = movie.finish().unwrap();
    let Some(r) = common::render_swf("alpha0_reveal", &swf) else { return };
    save("alpha0_reveal.ruffle", &r);
    r.assert_near(40, 40, [255, 0, 0], 4, "alpha 0 fill with alpha offset 255");
}
