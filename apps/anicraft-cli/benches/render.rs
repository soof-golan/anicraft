//! Frame benchmarks through the command-line glue: open a document, evaluate a frame, rasterise it.
//!
//! Cases are the hand-written verification fixtures (frame 1) and, when `ANICRAFT_SAMPLES` is set, frame 1200 of the
//! unicorn sample (the frame that exposed the nested group matrix bug) and a frame of the derwood sample. Each case
//! prints a `checksum` of its pixels before timing; compare those lines between two runs to see that an optimisation
//! left the output unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "benchmark code")]

use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::path::Path;

use anicraft_cli::export::{RenderSpec, render_frame};
use anicraft_cli::project::Project;
use criterion::{Criterion, criterion_group, criterion_main};

const FIXTURES: [&str; 10] = ["nest_skew", "matrices", "group_abs", "masks", "blend", "color_fx", "filters", "tweens", "scenes", "text_rtl"];

fn pixel_digest(project: &Project, frame: u32) -> u64 {
    let mut ev = project.evaluator(false);
    let scene = project.scene_index(None).unwrap();
    let image = render_frame(&mut ev, scene, frame, RenderSpec::default()).unwrap();
    let mut hasher = DefaultHasher::new();
    image.premultiplied_rgba.hash(&mut hasher);
    hasher.finish()
}

fn bench_case(c: &mut Criterion, name: &str, path: &Path, frame: u32) {
    let project = Project::open(path).unwrap();
    println!("checksum {name}: frame {frame} pixels {:016x}", pixel_digest(&project, frame));
    let scene = project.scene_index(None).unwrap();
    // A fresh evaluator per iteration: decoded bitmaps and symbol caches start cold, as in `anicraft-cli render`.
    c.bench_function(&format!("render_cold/{name}"), |b| {
        b.iter(|| {
            let mut ev = project.evaluator(false);
            black_box(render_frame(&mut ev, scene, frame, RenderSpec::default()).unwrap())
        });
    });
    // One evaluator for all iterations: the steady state of an export.
    let mut ev = project.evaluator(false);
    c.bench_function(&format!("render_warm/{name}"), |b| b.iter(|| black_box(render_frame(&mut ev, scene, frame, RenderSpec::default()).unwrap())));
}

fn fixtures(c: &mut Criterion) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visual");
    for name in FIXTURES {
        bench_case(c, name, &root.join(name), 0);
    }
}

fn samples(c: &mut Criterion) {
    if anim_testkit::samples_dir().is_none() {
        println!("skipping the samples: ANICRAFT_SAMPLES is unset");
        return;
    }
    for (name, file, frame) in [("unicorn_1200", "unicorn_4AE.fla", 1199), ("derwood_1", "derwood_green_choreo_short.fla", 0)] {
        if let Some(path) = anim_testkit::sample(file) {
            bench_case(c, name, &path, frame);
        }
    }
}

criterion_group!(benches, fixtures, samples);
criterion_main!(benches);
