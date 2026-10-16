//! Reader and writer benchmarks: the Adobe sample files (skipped when `ANICRAFT_SAMPLES` is unset) and a synthetic
//! stress document with thousands of keyframes, shapes and nested symbols.
//!
//! Each case prints a `checksum` line before timing. Compare those lines between two runs to see that an
//! optimisation left the output unchanged.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::format_push_string,
    clippy::missing_const_for_fn,
    reason = "benchmark code over synthetic documents"
)]

use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::path::PathBuf;

use anicraft_xfl::{Loaded, ReadOptions, WriteOptions, read_bytes, read_with, write_fla_bytes};
use criterion::{Criterion, criterion_group, criterion_main};

fn sample(name: &str) -> Option<PathBuf> {
    let root = std::env::var_os("ANICRAFT_SAMPLES")?;
    let path = PathBuf::from(root).join(name);
    path.exists().then_some(path)
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// A polygon with `n` line segments and a few quadratic curves, in XFL edge syntax.
fn edge_text(n: usize, seed: usize) -> String {
    let mut text = String::from("!0 0");
    for i in 1..=n {
        let x = (i * 37 + seed * 11) % 400;
        let y = (i * 53 + seed * 7) % 300;
        if i % 7 == 0 {
            let _ = write!(text, "[{x} {y} {} {}", x + 5, y + 5);
        } else {
            let _ = write!(text, "|{x} {y}");
        }
    }
    text.push_str("|0 0");
    text
}

fn shape(n: usize, seed: usize) -> String {
    format!(
        "<DOMShape><fills><FillStyle index=\"1\"><SolidColor color=\"#{:06X}\"/></FillStyle></fills>\
         <strokes><StrokeStyle index=\"1\"><SolidStroke scaleMode=\"normal\" weight=\"2\"><fill><SolidColor color=\"#000000\"/></fill></SolidStroke></StrokeStyle></strokes>\
         <edges><Edge fillStyle1=\"1\" strokeStyle=\"1\" edges=\"{}\"/></edges></DOMShape>",
        (seed * 2_654_435) & 0x00FF_FFFF,
        edge_text(n, seed)
    )
}

fn instance(name: &str, k: usize) -> String {
    format!(
        "<DOMSymbolInstance libraryItemName=\"{name}\" symbolType=\"graphic\" loop=\"loop\"><matrix>\
         <Matrix a=\"0.9\" b=\"0.1\" c=\"-0.1\" d=\"-0.9\" tx=\"{k}\" ty=\"{}\"/></matrix>\
         <color><Color tintMultiplier=\"0.3\" tintColor=\"#FF8800\"/></color>\
         <transformationPoint><Point x=\"3\" y=\"4\"/></transformationPoint></DOMSymbolInstance>",
        k % 50
    )
}

fn layer(name: &str, frames: usize, body: &dyn Fn(usize) -> String) -> String {
    let mut out = format!("<DOMLayer name=\"{name}\" color=\"#4F80FF\"><frames>");
    for f in 0..frames {
        let _ = write!(
            out,
            "<DOMFrame index=\"{f}\" duration=\"1\" keyMode=\"9728\"><elements>{}</elements></DOMFrame>",
            body(f)
        );
    }
    out.push_str("</frames></DOMLayer>");
    out
}

/// `symbols` library symbols (each with `layers` layers of `frames` keyframes, nested one level on the previous
/// symbol) and a scene that places them all. Returns the folder contents as (path, text).
fn synthetic(symbols: usize, layers: usize, frames: usize, edges: usize) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut includes = String::new();
    for s in 0..symbols {
        let name = format!("Sym{s}");
        let mut layer_xml = String::new();
        for l in 0..layers {
            layer_xml.push_str(&layer(&format!("L{l}"), frames, &|f| {
                let mut body = shape(edges, s * 1000 + l * 10 + f);
                if s > 0 && l == 0 {
                    body.push_str(&instance(&format!("Sym{}", s - 1), f));
                }
                body
            }));
        }
        files.push((
            format!("LIBRARY/{name}.xml"),
            format!(
                "<DOMSymbolItem xmlns=\"http://ns.adobe.com/xfl/2008/\" name=\"{name}\" itemID=\"f0000001-{s:08x}\" symbolType=\"graphic\">\
                 <timeline><DOMTimeline name=\"{name}\"><layers>{layer_xml}</layers></DOMTimeline></timeline></DOMSymbolItem>"
            ),
        ));
        let _ = write!(includes, "<Include href=\"{name}.xml\" loadImmediate=\"false\" itemID=\"f0000001-{s:08x}\"/>");
    }
    let scene_layers: String = (0..layers)
        .map(|l| layer(&format!("S{l}"), frames, &|f| instance(&format!("Sym{}", (l + f) % symbols), f)))
        .collect();
    files.push((
        "DOMDocument.xml".to_owned(),
        format!(
            "<DOMDocument xmlns=\"http://ns.adobe.com/xfl/2008/\" width=\"400\" height=\"300\" frameRate=\"24\" currentTimeline=\"1\" xflVersion=\"2.9\">\
             <symbols>{includes}</symbols><timelines><DOMTimeline name=\"Scene 1\"><layers>{scene_layers}</layers></DOMTimeline></timelines></DOMDocument>"
        ),
    ));
    files
}

fn write_folder(files: &[(String, String)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text).unwrap();
    }
    dir
}

fn report(label: &str, loaded: &Loaded) {
    let summary = loaded.summary(0);
    let (bytes, _) = write_fla_bytes(&loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default()).unwrap();
    println!(
        "checksum {label}: {} layers, {} keyframes, {} elements, {} symbols, {} warnings, written {} bytes digest {:016x}",
        summary.layers,
        summary.keyframes,
        summary.elements,
        summary.symbols,
        summary.warnings,
        bytes.len(),
        digest(&bytes)
    );
}

fn samples(c: &mut Criterion) {
    let mut group = c.benchmark_group("samples");
    group.sample_size(10);
    for name in ["unicorn_4AE.fla", "derwood_green_choreo_short.fla"] {
        let Some(path) = sample(name) else {
            println!("skipping {name}: ANICRAFT_SAMPLES unset or file missing");
            continue;
        };
        let loaded = read_with(&path, &ReadOptions::default()).unwrap();
        report(name, &loaded);
        group.bench_function(format!("read/{name}"), |b| b.iter(|| black_box(read_with(&path, &ReadOptions::default()).unwrap())));
        let no_media = ReadOptions { load_media: false, ..ReadOptions::default() };
        group.bench_function(format!("read_no_media/{name}"), |b| b.iter(|| black_box(read_with(&path, &no_media).unwrap())));
        group.bench_function(format!("write/{name}"), |b| {
            b.iter(|| black_box(write_fla_bytes(&loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default()).unwrap()));
        });
    }
    group.finish();
}

fn synthetic_cases(c: &mut Criterion) {
    let mut group = c.benchmark_group("synthetic");
    group.sample_size(20);
    // 60 symbols x 12 layers x 40 keyframes, 120 edge points each, nested in a chain; the scene places them 12 x 40 times.
    let files = synthetic(60, 12, 40, 120);
    let total: usize = files.iter().map(|(_, text)| text.len()).sum();
    println!("synthetic document: {} files, {} bytes of XML", files.len(), total);
    let dir = write_folder(&files);
    let loaded = read_with(dir.path(), &ReadOptions::default()).unwrap();
    report("synthetic", &loaded);
    let (zip, _) = write_fla_bytes(&loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default()).unwrap();

    group.bench_function("read_folder", |b| b.iter(|| black_box(read_with(dir.path(), &ReadOptions::default()).unwrap())));
    group.bench_function("read_zip", |b| b.iter(|| black_box(read_bytes(zip.clone(), &ReadOptions::default()).unwrap())));
    group.bench_function("write_zip", |b| {
        b.iter(|| black_box(write_fla_bytes(&loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default()).unwrap()));
    });
    group.bench_function("document_xml", |b| b.iter(|| black_box(anicraft_xfl::write::document_xml(&loaded.document, &loaded.media))));
    group.finish();
}

criterion_group!(benches, samples, synthetic_cases);
criterion_main!(benches);
