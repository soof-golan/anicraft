//! Predict, render, inspect: the XFL reader and the renderer on hand-written fixtures. The predictions are in
//! `tests/fixtures/visual/<name>.expect.md`; every probe below comes from there, not from the output.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test code"
)]

mod common;

use std::path::{Path, PathBuf};

use common::{Pixels, cli, cli_ok, read_png};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visual").join(name)
}

/// Where renders for inspection go when `ANICRAFT_VISUAL_OUT` is set.
fn render(name: &str, args: &[&str], tag: &str) -> Pixels {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.png");
    let src = fixture(name);
    let mut full = vec!["render", src.to_str().unwrap(), "-o", out.to_str().unwrap()];
    full.extend_from_slice(args);
    cli_ok(&full);
    let img = read_png(&out);
    if let Ok(target) = std::env::var("ANICRAFT_VISUAL_OUT") {
        std::fs::create_dir_all(&target).unwrap();
        std::fs::copy(&out, Path::new(&target).join(format!("{tag}.png"))).unwrap();
        // Nearest-neighbour zoom for inspection with the image viewer: whole image at 6x.
        save_zoom(&img, (0, 0, img.width, img.height), 6, &Path::new(&target).join(format!("{tag}_x6.png")));
        // `ANICRAFT_VISUAL_CROP=x,y,w,h` also writes a 16x crop of every render.
        if let Ok(c) = std::env::var("ANICRAFT_VISUAL_CROP") {
            let v: Vec<u32> = c.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if let [x, y, w, h] = v[..] {
                save_zoom(&img, (x, y, w, h), 16, &Path::new(&target).join(format!("{tag}_crop.png")));
            }
        }
    }
    img
}

/// Crops `(x, y, w, h)` and scales it up by `k` with nearest neighbour, so single pixels stay visible.
fn save_zoom(img: &Pixels, (x0, y0, w, h): (u32, u32, u32, u32), k: u32, path: &Path) {
    let (w, h) = (w.min(img.width.saturating_sub(x0)), h.min(img.height.saturating_sub(y0)));
    let mut rgba = Vec::new();
    for y in 0..h * k {
        for x in 0..w * k {
            rgba.extend_from_slice(&img.at(x0 + x / k, y0 + y / k));
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w * k, h * k);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
}

#[track_caller]
fn is(img: &Pixels, x: u32, y: u32, want: [u8; 3], tol: u8) {
    let got = img.at(x, y);
    for i in 0..3 {
        assert!(got[i].abs_diff(want[i]) <= tol, "pixel ({x},{y}) is {got:?}, want {want:?} +-{tol}");
    }
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn nested_symbols_with_skew_mirror_and_far_transformation_points() {
    let img = render("nest_skew", &[], "nest_skew");
    assert_eq!((img.width, img.height), (100, 100));
    // Rows of the parallelogram: row y=15 spans X 25..35, row y=19 spans X 29..39.
    for (x, want) in [(24, WHITE), (26, BLUE), (34, BLUE), (36, WHITE)] {
        is(&img, x, 15, want, 0);
    }
    for (x, want) in [(28, WHITE), (30, BLUE), (38, BLUE), (40, WHITE)] {
        is(&img, x, 19, want, 0);
    }
    // The marker is at the top right after the mirror.
    is(&img, 30, 11, RED, 0);
    is(&img, 22, 11, BLUE, 0);
    for (x, y) in [(35, 12), (15, 15), (30, 25)] {
        is(&img, x, y, WHITE, 0);
    }
}

#[test]
fn rotations_reflections_and_stretches_ignore_the_transformation_point() {
    let img = render("matrices", &[], "matrices");
    for (x, y, want) in [
        (45, 12, BLUE),
        (45, 24, BLUE),
        (45, 28, RED),
        (45, 31, WHITE),
        (39, 20, WHITE),
        (51, 20, WHITE),
        (88, 85, BLUE),
        (80, 85, BLUE),
        (72, 85, RED),
        (91, 85, WHITE),
        (80, 79, WHITE),
        (15, 60, BLUE),
        (28, 60, RED),
        (15, 45, WHITE),
        (15, 72, WHITE),
        (12, 52, BLUE),
        (12, 68, BLUE),
    ] {
        is(&img, x, y, want, 0);
    }
}

#[test]
fn group_members_keep_their_own_matrix() {
    let img = render("group_abs", &[], "group_abs");
    is(&img, 30, 30, RED, 0);
    is(&img, 20, 20, RED, 0);
    is(&img, 70, 30, BLUE, 0);
    is(&img, 49, 30, RED, 0);
    is(&img, 50, 30, BLUE, 0);
    is(&img, 10, 65, [0, 255, 0], 0);
    is(&img, 10, 55, WHITE, 0);
    is(&img, 10, 75, WHITE, 0);
    is(&img, 75, 65, [255, 128, 0], 1);
    is(&img, 55, 65, WHITE, 0);
    is(&img, 95, 95, WHITE, 0);
}

#[test]
fn masks_guides_hidden_layers_and_masks_inside_scaled_symbols() {
    let img = render("masks", &[], "masks");
    for (x, y) in [(25, 45), (55, 45), (40, 55), (25, 58), (80, 25), (65, 10), (98, 43)] {
        is(&img, x, y, RED, 0);
    }
    for (x, y) in [(40, 45), (25, 25), (40, 35), (65, 50), (25, 65), (10, 50), (5, 5), (50, 95), (15, 90), (55, 25), (80, 50), (80, 3), (61, 45)] {
        is(&img, x, y, WHITE, 0);
    }
    is(&img, 59, 45, RED, 0);
    is(&img, 80, 90, [255, 255, 0], 0);
}

#[test]
fn blend_modes_match_the_formulas() {
    let img = render("blend", &[], "blend");
    let table: [[(u32, u32); 3]; 3] = [[(0, 0), (1, 0), (2, 0)], [(0, 1), (1, 1), (2, 1)], [(0, 2), (1, 2), (2, 2)]];
    let want = [
        ((0, 0), [78, 59, 39]),
        ((1, 0), [222, 191, 211]),
        ((2, 0), [100, 50, 150]),
        ((0, 1), [200, 150, 200]),
        ((1, 1), [255, 250, 250]),
        ((2, 1), [100, 0, 0]),
        ((0, 2), [100, 100, 50]),
        ((1, 2), [188, 118, 78]),
        ((2, 2), [157, 127, 167]),
    ];
    assert_eq!(table.len(), 3);
    for ((i, j), rgb) in want {
        is(&img, 30 * i + 10, 30 * j + 10, rgb, 3);
    }
    is(&img, 25, 25, [200, 100, 50], 1);
}

#[test]
fn colour_effects_alpha_and_visibility() {
    let img = render("color_fx", &[], "color_fx");
    is(&img, 10, 10, [128, 0, 128], 2);
    is(&img, 40, 10, [128, 128, 255], 2);
    is(&img, 70, 10, [0, 0, 128], 2);
    is(&img, 10, 40, [128, 128, 255], 2);
    is(&img, 40, 40, [255, 0, 128], 2);
    is(&img, 70, 40, [191, 191, 255], 2);
    is(&img, 10, 70, [255, 255, 0], 0);
    is(&img, 40, 70, WHITE, 0);
    is(&img, 70, 70, WHITE, 0);
    is(&img, 25, 25, WHITE, 0);
    is(&img, 55, 55, WHITE, 0);
}

#[test]
fn blur_halo_drop_shadow_and_hide_object() {
    let img = render("filters", &[], "filters");
    let r = |x: u32, y: u32| u32::from(img.at(x, y)[0]);
    // Pixel x covers x..x+1, so the pixel mirroring (35,25) about the square's centre 25 is (14,25), not (15,25).
    let sides = [r(14, 25), r(35, 25), r(25, 14), r(25, 35)];
    let (lo, hi) = (sides.iter().min().unwrap(), sides.iter().max().unwrap());
    assert!(hi - lo <= 4, "blur is symmetric: {sides:?}");
    assert!((120..=250).contains(lo), "{sides:?}");
    for (x, y) in [(5, 25), (45, 25), (25, 5), (25, 45)] {
        assert!(img.at(x, y)[..3].iter().all(|c| *c >= 253), "clean at ({x},{y})");
    }
    assert!(img.at(25, 25)[..3].iter().all(|c| (40..=170).contains(c)), "{:?}", img.at(25, 25));
    for x in 25..45 {
        assert!(r(x + 1, 25).abs_diff(r(x, 25)) < 60, "no hard crop at x={x}");
    }
    is(&img, 22, 62, RED, 0);
    for (x, y) in [(34, 74), (28, 72)] {
        is(&img, x, y, [0, 0, 0], 20);
    }
    for (x, y) in [(34, 65), (24, 75), (45, 75), (62, 62), (65, 65)] {
        is(&img, x, y, WHITE, 0);
    }
    is(&img, 72, 72, [0, 0, 0], 20);
    assert!(r(62, 20) < 250);
    assert!(r(99, 20) >= 250);
    is(&img, 75, 13, WHITE, 0);
    is(&img, 75, 28, WHITE, 0);
    assert!(r(65, 20).abs_diff(r(84, 20)) <= 4, "{} {}", r(65, 20), r(84, 20));
}

#[test]
fn classic_and_shape_tweens_across_the_rotation_wrap() {
    // `--frame` counts from 1; the prediction counts from 0.
    let at = |f: u32| render("tweens", &["--frame", &(f + 1).to_string()], &format!("tweens_f{f}"));
    let f0 = at(0);
    is(&f0, 65, 44, BLUE, 0);
    is(&f0, 50, 47, BLUE, 0);
    is(&f0, 65, 50, WHITE, 0);
    is(&f0, 70, 20, BLUE, 0);
    is(&f0, 50, 40, WHITE, 0);
    is(&f0, 20, 85, [0, 192, 0], 0);
    is(&f0, 40, 85, WHITE, 0);
    let f5 = at(5);
    is(&f5, 60, 50, BLUE, 0);
    is(&f5, 60, 44, WHITE, 0);
    is(&f5, 60, 56, WHITE, 0);
    is(&f5, 20, 50, WHITE, 0);
    is(&f5, 40, 10, BLUE, 0);
    is(&f5, 60, 30, WHITE, 0);
    is(&f5, 70, 20, WHITE, 0);
    is(&f5, 50, 85, [0, 192, 0], 0);
    is(&f5, 70, 85, WHITE, 0);
    is(&f5, 12, 85, [0, 192, 0], 0);
    is(&f5, 8, 85, WHITE, 0);
    let f10 = at(10);
    is(&f10, 65, 56, BLUE, 0);
    is(&f10, 65, 50, WHITE, 0);
    is(&f10, 50, 40, BLUE, 0);
    is(&f10, 70, 20, WHITE, 0);
    is(&f10, 80, 85, [0, 192, 0], 0);
    is(&f10, 92, 85, WHITE, 0);
}

#[test]
fn scenes_labels_and_the_background() {
    let default = render("scenes", &[], "scenes_default");
    is(&default, 25, 25, RED, 0);
    is(&default, 75, 25, [32, 32, 32], 0);
    is(&default, 95, 55, [32, 32, 32], 0);
    let first = |f: &str| render("scenes", &["--scene", "First", "--frame", f], &format!("scenes_first{f}"));
    let f1 = first("1");
    is(&f1, 25, 25, BLUE, 0);
    is(&f1, 75, 25, [32, 32, 32], 0);
    is(&first("2"), 25, 25, BLUE, 0);
    let f4 = first("4");
    is(&f4, 75, 25, [0, 255, 0], 0);
    is(&f4, 25, 25, [32, 32, 32], 0);
    is(&first("5"), 75, 25, [0, 255, 0], 0);
    let src = fixture("scenes");
    let past = cli(&["render", src.to_str().unwrap(), "--scene", "First", "--frame", "6", "-o", "/nonexistent-dir/x.png"]);
    assert!(!past.status.success());
    assert!(!String::from_utf8_lossy(&past.stderr).contains("panicked"));
}

/// read -> write -> read stability, judged on pixels: converting each fixture to a zipped FLA and to a rewritten
/// XFL folder must not change a single pixel of any frame.
#[test]
fn rewriting_a_fixture_does_not_change_its_renders() {
    let dir = tempfile::tempdir().unwrap();
    for (name, frames) in [
        ("nest_skew", vec![1]),
        ("matrices", vec![1]),
        ("group_abs", vec![1]),
        ("masks", vec![1]),
        ("blend", vec![1]),
        ("color_fx", vec![1]),
        ("filters", vec![1]),
        ("tweens", vec![1, 4, 6, 11]),
        ("text_rtl", vec![1]),
        ("scenes", vec![1, 2]),
    ] {
        let src = fixture(name);
        let fla = dir.path().join(format!("{name}.fla"));
        let folder = dir.path().join(format!("{name}_again"));
        cli_ok(&["convert", src.to_str().unwrap(), fla.to_str().unwrap()]);
        cli_ok(&["convert", fla.to_str().unwrap(), folder.to_str().unwrap()]);
        for frame in frames {
            let f = frame.to_string();
            let mut base = vec!["render", src.to_str().unwrap(), "--frame", &f];
            if name == "scenes" {
                base.extend(["--scene", "First"]);
            }
            let shot = |input: &Path, tag: &str| {
                let out = dir.path().join(format!("{name}_{frame}_{tag}.png"));
                let mut args = base.clone();
                args[1] = input.to_str().unwrap();
                args.extend(["-o", out.to_str().unwrap()]);
                cli_ok(&args);
                read_png(&out)
            };
            let original = shot(&src, "orig");
            for (input, tag) in [(&fla, "fla"), (&folder, "folder")] {
                let again = shot(input, tag);
                assert_eq!(original.rgba, again.rgba, "{name} frame {frame} changed after a rewrite through {tag}");
            }
        }
    }
}

/// The goldens are renders that were inspected and found to match their predictions; they must stay within 2 levels.
#[test]
fn goldens_stay_put() {
    for (name, args, golden) in [
        ("nest_skew", vec![], "nest_skew"),
        ("matrices", vec![], "matrices"),
        ("group_abs", vec![], "group_abs"),
        ("masks", vec![], "masks"),
        ("blend", vec![], "blend"),
        ("color_fx", vec![], "color_fx"),
        ("tweens", vec!["--frame", "6"], "tweens_f5"),
    ] {
        let now = render(name, &args, &format!("golden_{golden}"));
        let want = read_png(&fixture("golden").join(format!("{golden}.png")));
        assert_eq!((now.width, now.height), (want.width, want.height), "{golden}");
        let worst = now.rgba.iter().zip(&want.rgba).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 2, "{golden} drifted by {worst} levels");
    }
}

/// Mean x of the pixels that satisfy `pick`, and how many there are.
fn centroid(img: &Pixels, y0: u32, y1: u32, pick: impl Fn([u8; 4]) -> bool) -> Option<(f64, usize)> {
    let (mut sum, mut n) = (0.0, 0_usize);
    for y in y0..y1 {
        for x in 0..img.width {
            if pick(img.at(x, y)) {
                sum += f64::from(x);
                n += 1;
            }
        }
    }
    (n > 0).then(|| (sum / n as f64, n))
}

#[test]
fn bidi_runs_are_ordered_by_the_unicode_algorithm() {
    let img = render("text_rtl", &[], "text_rtl");
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 80 && p[2] < 80;
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 80 && p[1] < 80;
    let green = |p: [u8; 4]| p[1] > 120 && p[0] < 80 && p[2] < 80;
    let c = |y0, y1, f: &dyn Fn([u8; 4]) -> bool| centroid(&img, y0, y1, f).unwrap();
    let (g1, gn) = c(0, 38, &green);
    let (b1, bn) = c(0, 38, &blue);
    let (r1, rn) = c(0, 38, &red);
    assert!(gn >= 20 && bn >= 20 && rn >= 100, "ink: {gn} {bn} {rn}");
    assert!(g1 < b1 && b1 < r1, "field 1: {g1} {b1} {r1}");
    let (g2, _) = c(38, 72, &green);
    let (r2, _) = c(38, 72, &red);
    let (b2, _) = c(38, 72, &blue);
    assert!(g2 < r2 && r2 < b2, "field 2: {g2} {r2} {b2}");
    let (r3, _) = c(72, 110, &red);
    let (b3, _) = c(72, 110, &blue);
    assert!(b3 < r3, "field 3: {b3} {r3}");
}
