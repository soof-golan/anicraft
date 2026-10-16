//! The command-line tool on our own fixture: every command and every export format.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    reason = "test code"
)]

use crate::common;

use common::{cli, cli_ok, mover, read_png};
use serde_json::Value;

fn path(dir: &tempfile::TempDir, name: &str) -> String {
    dir.path().join(name).to_str().unwrap().to_owned()
}

#[test]
fn info_describes_the_document_as_json_and_text() {
    let fixture = mover();
    let json = cli_ok(&["info", fixture.to_str().unwrap(), "--json"]);
    let report: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(report["stage"]["width"], 120.0);
    assert_eq!(report["stage"]["fps"], 12.0);
    assert_eq!(report["scenes"][0]["frames"], 12);
    assert_eq!(report["scenes"][0]["layers"][0]["name"], "Mover");
    assert_eq!(report["library"]["symbols"][0]["name"], "Box");
    assert_eq!(report["warnings"].as_array().unwrap().len(), 0);

    let text = cli_ok(&["info", fixture.to_str().unwrap(), "--layers"]);
    assert!(text.contains("stage 120x80 px at 12 fps"), "{text}");
    assert!(text.contains("\"Mover\""), "{text}");
    assert!(text.contains("1 symbol(s)"), "{text}");
}

#[test]
fn render_moves_the_square_across_the_stage() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = mover();
    let mut centres = Vec::new();
    for frame in ["1", "6", "12"] {
        let out = path(&dir, &format!("f{frame}.png"));
        cli_ok(&["render", fixture.to_str().unwrap(), "--frame", frame, "-o", &out]);
        let img = read_png(&dir.path().join(format!("f{frame}.png")));
        assert_eq!((img.width, img.height), (120, 80));
        assert_eq!(img.at(115, 75), [255, 255, 255, 255], "background is the stage colour");
        let cols = img.red_columns();
        assert!(!cols.is_empty(), "frame {frame} shows the square");
        centres.push(u32::midpoint(cols[0], cols[cols.len() - 1]));
    }
    // Frame 1 is the first keyframe (x 10..30) and frame 12 the last (x 90..110). Frame 6 is the last tweened frame,
    // five sixths of the way: x = 10 + 80 * 5 / 6, so the centre is about 87.
    assert!((18..=22).contains(&centres[0]), "{centres:?}");
    assert!((84..=90).contains(&centres[1]), "{centres:?}");
    assert!((98..=102).contains(&centres[2]), "{centres:?}");
}

#[test]
fn render_scales_and_can_leave_out_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let out = path(&dir, "big.png");
    cli_ok(&["render", mover().to_str().unwrap(), "--frame", "1", "--scale", "2", "--transparent", "-o", &out]);
    let img = read_png(&dir.path().join("big.png"));
    assert_eq!((img.width, img.height), (240, 160));
    assert_eq!(img.at(235, 155)[3], 0, "transparent background");
}

#[test]
fn render_a_library_symbol_crops_to_its_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let out = path(&dir, "box.png");
    cli_ok(&["render", mover().to_str().unwrap(), "--symbol", "Box", "-o", &out]);
    let img = read_png(&dir.path().join("box.png"));
    // 20 x 20 px square plus an 8 px margin on every side.
    assert_eq!((img.width, img.height), (36, 36));
    assert_eq!(img.at(18, 18)[0], 255);
}

#[test]
fn bad_arguments_fail_with_a_helpful_message() {
    let fixture = mover();
    let f = fixture.to_str().unwrap();
    let out = cli(&["render", f, "--frame", "0", "-o", "unused.png"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("start at 1"));
    let out = cli(&["render", f, "--frame", "13", "-o", "unused.png"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("beyond the end"));
    let out = cli(&["render", f, "--scene", "Nope", "-o", "unused.png"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("Scene 1"));
    let out = cli(&["render", f, "--scale", "-1", "-o", "unused.png"]);
    assert!(!out.status.success());
    let out = cli(&["info", "/definitely/not/here.fla"]);
    assert!(!out.status.success());
    let out = cli(&["export", f, "--format", "teapot", "-o", "x"]);
    assert!(!out.status.success());
}

#[test]
fn every_export_format_writes_a_valid_file() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = mover();
    let f = fixture.to_str().unwrap();

    let seq = path(&dir, "seq");
    cli_ok(&["export", f, "--format", "png-seq", "-o", &seq, "--from", "2", "--to", "4", "-q"]);
    let mut names: Vec<_> = std::fs::read_dir(&seq).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["frame_0001.png", "frame_0002.png", "frame_0003.png"]);

    let gif = path(&dir, "a.gif");
    cli_ok(&["export", f, "--format", "gif", "-o", &gif, "-q"]);
    let bytes = std::fs::read(&gif).unwrap();
    assert!(bytes.starts_with(b"GIF89a"));

    let svg = path(&dir, "a.svg");
    cli_ok(&["export", f, "--format", "svg", "--from", "6", "-o", &svg, "-q"]);
    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(text.contains("<svg") && text.contains("</svg>"));

    let anim = path(&dir, "anim.svg");
    cli_ok(&["export", f, "--format", "svg-anim", "-o", &anim, "-q"]);
    assert!(std::fs::read_to_string(&anim).unwrap().contains("<animate"));

    let sheet = path(&dir, "sheet.png");
    cli_ok(&["export", f, "--format", "spritesheet", "-o", &sheet, "-q"]);
    assert!(read_png(&dir.path().join("sheet.png")).width >= 120);
    let json: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("sheet.json")).unwrap()).unwrap();
    assert_eq!(json["frames"].as_object().unwrap().len(), 12);

    let swf = path(&dir, "a.swf");
    cli_ok(&["export", f, "--format", "swf", "-o", &swf, "-q"]);
    let bytes = std::fs::read(&swf).unwrap();
    assert!(bytes.starts_with(b"FWS") || bytes.starts_with(b"CWS"));

    let mp4 = path(&dir, "a.mp4");
    cli_ok(&["export", f, "--format", "mp4", "-o", &mp4, "-q"]);
    let bytes = std::fs::read(&mp4).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");

    let mov = path(&dir, "a.mov");
    cli_ok(&["export", f, "--format", "mov", "--alpha", "-o", &mov, "-q"]);
    let bytes = std::fs::read(&mov).unwrap();
    assert!(bytes.windows(4).take(64).any(|w| w == b"ftyp" || w == b"moov" || w == b"mdat" || w == b"wide"), "a QuickTime container");
}

#[test]
fn parallel_png_sequence_matches_single_frame_renders_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = mover();
    let f = fixture.to_str().unwrap();
    // Every frame is rendered on whichever worker is free: any reordering or shared state between workers would change a
    // file relative to rendering that frame alone.
    let seq = path(&dir, "seq");
    cli_ok(&["export", f, "--format", "png-seq", "-o", &seq, "-q"]);
    for frame in 1..=12 {
        let single = path(&dir, "single.png");
        cli_ok(&["render", f, "--frame", &frame.to_string(), "-o", &single]);
        let from_sequence = dir.path().join("seq").join(format!("frame_{frame:04}.png"));
        assert_eq!(std::fs::read(from_sequence).unwrap(), std::fs::read(dir.path().join("single.png")).unwrap(), "frame {frame}");
    }
}

#[test]
fn export_resamples_to_another_frame_rate() {
    let dir = tempfile::tempdir().unwrap();
    let seq = path(&dir, "seq");
    let out = cli_ok(&["export", mover().to_str().unwrap(), "--format", "png-seq", "--fps", "24", "-o", &seq, "-q"]);
    assert!(out.contains("24 frame(s)"), "{out}");
    assert_eq!(std::fs::read_dir(&seq).unwrap().count(), 24);
}

#[test]
fn convert_round_trips_through_fla_and_xfl_folders() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = mover();
    let fla = path(&dir, "out.fla");
    cli_ok(&["convert", fixture.to_str().unwrap(), &fla]);
    let again = path(&dir, "again.xfl");
    cli_ok(&["convert", &fla, &again]);
    assert!(dir.path().join("again.xfl/DOMDocument.xml").is_file());
    assert!(dir.path().join("again.xfl/LIBRARY/Box.xml").is_file());

    // The copies describe the same document.
    let original: Value = serde_json::from_str(&cli_ok(&["info", fixture.to_str().unwrap(), "--json"])).unwrap();
    for copy in [&fla, &again] {
        let report: Value = serde_json::from_str(&cli_ok(&["info", copy, "--json"])).unwrap();
        assert_eq!(report["scenes"], original["scenes"], "{copy}");
        assert_eq!(report["library"], original["library"], "{copy}");
        assert_eq!(report["stage"], original["stage"], "{copy}");
    }
    // And render the same pixels.
    let a = path(&dir, "a.png");
    let b = path(&dir, "b.png");
    cli_ok(&["render", fixture.to_str().unwrap(), "--frame", "7", "-o", &a]);
    cli_ok(&["render", &fla, "--frame", "7", "-o", &b]);
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    assert!(!cli(&["convert", &fla, &fla]).status.success(), "same path is refused");
}
