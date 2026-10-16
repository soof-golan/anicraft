//! Exporter benchmarks: frames evaluated from the Adobe sample files (skipped when `ANICRAFT_SAMPLES` is unset) and
//! synthetic stress display lists built from the hardest verification fixtures, scaled up.
//!
//! Each case prints a `checksum` line before timing. Compare those lines between two runs to see that an
//! optimisation left the SWF bytes unchanged.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    clippy::print_stderr,
    clippy::missing_const_for_fn,
    clippy::suboptimal_flops,
    clippy::similar_names,
    reason = "benchmark code over synthetic documents"
)]

use std::f64::consts::TAU;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::path::PathBuf;

use anicraft_swf::{ExportOptions, ExportOutput, export_display_lists};
use anicraft_xfl::{ReadOptions, read_with};
use anim_core::kurbo::{BezPath, Circle, Rect, Shape};
use anim_core::{Affine, BlendMode, ColorTransform, Rgba8, Size};
use anim_display::{
    Cap, DisplayList, Fill, FillRule, Filter, Gradient, GradientStop, Group, ImageStore, Interpolation, Join, Node, Paint, Spread, Stroke, StrokeScale,
    StrokeStyle,
};
use anim_timeline::{EvalOptions, Evaluator};
use criterion::{Criterion, criterion_group, criterion_main};

fn sample(name: &str) -> Option<PathBuf> {
    let root = std::env::var_os("ANICRAFT_SAMPLES")?;
    let path = PathBuf::from(root).join(name);
    path.exists().then_some(path)
}

fn checksum(output: &ExportOutput) -> u64 {
    let mut hasher = DefaultHasher::new();
    output.swf.hash(&mut hasher);
    hasher.finish()
}

fn count_nodes(nodes: &[Node]) -> usize {
    nodes.iter().map(|n| if let Node::Group(g) = n { 1 + count_nodes(&g.children) + g.mask.as_deref().map_or(0, count_nodes) } else { 1 }).sum()
}

/// Exports once to print the checksum, then times the export.
fn bench_export(c: &mut Criterion, group: &str, name: &str, lists: &[DisplayList], images: &ImageStore) {
    let options = || ExportOptions::for_list(&lists[0], 24.0);
    let once = export_display_lists(lists.iter(), images, options()).unwrap();
    let nodes: usize = lists.iter().map(|l| count_nodes(&l.nodes)).sum();
    eprintln!("checksum {group}/{name}: {:016x} ({} bytes, {} frames, {nodes} nodes)", checksum(&once), once.swf.len(), lists.len());
    let mut g = c.benchmark_group(group);
    g.sample_size(10);
    g.bench_function(name, |b| b.iter(|| black_box(export_display_lists(black_box(lists).iter(), images, options()).unwrap())));
    g.finish();
}

// ---------------------------------------------------------------------------------------------------------------
// synthetic documents

/// A wobbly closed outline with `n` points: lines and quadratic and cubic curves, from a seed.
fn blob(n: usize, seed: usize, size: f64) -> BezPath {
    let at = |i: usize| {
        let angle = TAU * i as f64 / n as f64;
        let wobble = 0.6 + 0.4 * (((i * 37 + seed * 11) % 17) as f64 / 17.0);
        (size * wobble * angle.cos(), size * wobble * angle.sin())
    };
    let mut path = BezPath::new();
    path.move_to(at(0));
    for i in 1..n {
        match i % 3 {
            0 => path.line_to(at(i)),
            1 => path.quad_to(at(i), at((i + 1) % n)),
            _ => path.curve_to(at(i), at((i + 1) % n), at((i + 2) % n)),
        }
    }
    path.close_path();
    path
}

fn stops(count: usize) -> Vec<GradientStop> {
    (0..count)
        .map(|i| {
            let t = i as f32 / (count - 1) as f32;
            GradientStop { offset: t, color: Rgba8::new((t * 255.0) as u8, (255.0 - t * 255.0) as u8, 128, 200 + (i % 56) as u8) }
        })
        .collect()
}

fn gradient(seed: usize, radial_focal: f64) -> Gradient {
    Gradient {
        transform: Affine::scale(0.05 + (seed % 5) as f64 * 0.01),
        stops: stops(2 + seed % 6),
        spread: [Spread::Pad, Spread::Reflect, Spread::Repeat][seed % 3],
        interpolation: if seed.is_multiple_of(2) { Interpolation::Rgb } else { Interpolation::LinearRgb },
        focal: radial_focal,
    }
}

fn paint(seed: usize) -> Paint {
    match seed % 4 {
        0 => Paint::Solid(Rgba8::opaque((seed * 7) as u8, (seed * 13) as u8, (seed * 29) as u8)),
        1 => Paint::Linear(gradient(seed, 0.0)),
        2 => Paint::Radial(gradient(seed, 0.4)),
        _ => Paint::Solid(Rgba8::new(200, 40, (seed * 3) as u8, 140)),
    }
}

fn fill(path: BezPath, paint: Paint, rule: FillRule) -> Node {
    Node::Fill(Fill { transform: Affine::IDENTITY, path, rule, paint })
}

fn stroke(path: BezPath, width: f64, scale: StrokeScale, dashes: Vec<f64>, paint: Paint) -> Node {
    Node::Stroke(Stroke {
        transform: Affine::IDENTITY,
        path,
        style: StrokeStyle { width, cap: Cap::Round, join: Join::Miter, miter_limit: 3.0, scale, dashes },
        paint,
    })
}

fn list(nodes: Vec<Node>) -> DisplayList {
    DisplayList { size: Size::new(1920.0, 1080.0), background: Some(Rgba8::WHITE), nodes }
}

fn group(children: Vec<Node>, transform: Affine) -> Group {
    Group { transform, children, ..Group::default() }
}

/// `shapes` distinct filled and stroked shapes, each moved a little every frame: every shape is defined once and then
/// only placed, which is the common case for animations.
fn moving_shapes(shapes: usize, frames: usize) -> Vec<DisplayList> {
    (0..frames)
        .map(|f| {
            let nodes = (0..shapes)
                .map(|s| {
                    let x = 60.0 + (s % 40) as f64 * 45.0 + (f as f64 * 0.3 + s as f64).sin() * 6.0;
                    let y = 60.0 + (s / 40) as f64 * 45.0 + (f as f64 * 0.2 + s as f64).cos() * 6.0;
                    let at = Affine::translate((x, y)) * Affine::rotate(f as f64 * 0.02);
                    let body = if s % 3 == 0 {
                        stroke(blob(18, s, 18.0), 2.5, [StrokeScale::Normal, StrokeScale::None, StrokeScale::Horizontal][s % 3], vec![], paint(s))
                    } else {
                        fill(blob(24, s, 18.0), paint(s), if s % 5 == 0 { FillRule::EvenOdd } else { FillRule::NonZero })
                    };
                    Node::Group(group(vec![body], at))
                })
                .collect();
            list(nodes)
        })
        .collect()
}

/// Shapes whose geometry changes every frame (a morph-like or hand-drawn animation): every frame defines new shapes.
fn morphing_shapes(shapes: usize, frames: usize) -> Vec<DisplayList> {
    (0..frames)
        .map(|f| {
            let nodes = (0..shapes)
                .map(|s| {
                    let body = blob(40, s + f * 31, 30.0 + (f % 7) as f64);
                    let moved = Affine::translate((80.0 + (s % 20) as f64 * 90.0, 80.0 + (s / 20) as f64 * 90.0));
                    Node::Group(group(vec![fill(body, paint(s), FillRule::NonZero)], moved))
                })
                .collect();
            list(nodes)
        })
        .collect()
}

/// Long dashed strokes with a gradient: the dash splitter and the stroke-to-fill conversion dominate.
fn dashed_strokes(count: usize, frames: usize) -> Vec<DisplayList> {
    (0..frames)
        .map(|f| {
            let nodes = (0..count)
                .map(|s| {
                    let mut path = blob(60, s + f, 400.0);
                    path.extend(Circle::new((0.0, 0.0), 300.0 + (s % 9) as f64 * 5.0).to_path(0.1).iter());
                    let dashes = vec![2.0 + (s % 3) as f64, 3.0, 1.0, 3.0];
                    let at = Affine::translate((960.0, 540.0)) * Affine::scale(0.4 + (s % 5) as f64 * 0.1);
                    Node::Group(group(vec![stroke(path, 1.5, StrokeScale::Normal, dashes, paint(s + 1))], at))
                })
                .collect();
            list(nodes)
        })
        .collect()
}

fn mask_node(seed: usize) -> Vec<Node> {
    vec![fill(Circle::new((0.0, 0.0), 40.0 + (seed % 5) as f64 * 8.0).to_path(0.1), Paint::Solid(Rgba8::BLACK), FillRule::NonZero)]
}

fn nested(depth: usize, branch: usize, seed: usize) -> Group {
    let mut g = group(vec![], Affine::translate((6.0 * (seed % 5) as f64, 4.0)) * Affine::rotate(0.05 * (seed % 7) as f64) * Affine::scale(0.93));
    g.color = ColorTransform { mul: [0.97, 0.98, 1.0, 0.98], add: [2.0 / 255.0, 0.0, 0.0, 0.0] };
    g.blend = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay][seed % 4];
    if seed % 5 == 1 {
        g.filters = vec![Filter::Blur { x: 4.0, y: 4.0, quality: 1 }];
    }
    if seed % 4 == 2 {
        g.mask = Some(mask_node(seed));
    }
    g.children.push(fill(Rect::new(-30.0, -30.0, 30.0, 30.0).to_path(0.1), paint(seed), FillRule::NonZero));
    if depth > 0 {
        for b in 0..branch {
            g.children.push(Node::Group(nested(depth - 1, branch, seed * branch + b + 1)));
        }
    }
    g
}

/// Sprites in sprites with masks, filters, blend modes and colour transforms, over several frames.
fn nested_sprites(depth: usize, branch: usize, frames: usize) -> Vec<DisplayList> {
    (0..frames)
        .map(|f| {
            let mut g = nested(depth, branch, 0);
            g.transform = Affine::translate((960.0 + f as f64 * 3.0, 540.0)) * Affine::scale(2.0);
            list(vec![Node::Group(g)])
        })
        .collect()
}

fn synthetic(c: &mut Criterion) {
    let images = ImageStore::new();
    bench_export(c, "stress", "moving_shapes_800x60", &moving_shapes(800, 60), &images);
    bench_export(c, "stress", "morphing_shapes_300x40", &morphing_shapes(300, 40), &images);
    bench_export(c, "stress", "dashed_strokes_120x6", &dashed_strokes(120, 6), &images);
    bench_export(c, "stress", "nested_sprites_d6b3x8", &nested_sprites(6, 3, 8), &images);
}

// ---------------------------------------------------------------------------------------------------------------
// the sample files

fn samples(c: &mut Criterion) {
    for (name, frames) in [("unicorn_4AE.fla", 48), ("derwood_green_choreo_short.fla", 48)] {
        let Some(path) = sample(name) else {
            eprintln!("skipping {name}: ANICRAFT_SAMPLES is unset or the file is missing");
            continue;
        };
        let loaded = read_with(&path, &ReadOptions::default()).unwrap();
        let mut evaluator =
            Evaluator::new(&loaded.document).with_media(&loaded.media).with_options(EvalOptions { advance_movie_clips: true, ..EvalOptions::default() });
        let scene = loaded.document.current_timeline.min(loaded.document.scenes.len().saturating_sub(1));
        let total = loaded.document.scenes.get(scene).map_or(1, |s| s.frame_count().max(1));
        eprintln!("{name}: scene {scene} of {}, {total} frames", loaded.document.scenes.len());
        // the opening frames of a long animation are often empty: start at the busiest of a coarse scan
        let busiest = (0..total).step_by(25).max_by_key(|&f| count_nodes(&evaluator.frame(scene, f).nodes)).unwrap_or(0);
        let start = busiest.min(total.saturating_sub(frames));
        let lists: Vec<DisplayList> = (start..total.min(start + frames)).map(|f| evaluator.frame(scene, f)).collect();
        bench_export(c, "samples", name, &lists, evaluator.images());
    }
}

criterion_group!(benches, samples, synthetic);
criterion_main!(benches);
