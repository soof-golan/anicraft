//! Tests on the Adobe-authored sample files. They read `ANICRAFT_SAMPLES` and pass without doing anything when it is
//! unset. The samples are never copied into the repository; the rendered frames go to `$ANICRAFT_VISUAL_OUT` (when
//! set) so a person can look at them.
#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::expect_used, clippy::unwrap_used, clippy::panic, reason = "test code")]

mod common;

use std::path::{Path, PathBuf};

use common::{cli_ok, read_png};

fn sample(name: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("ANICRAFT_SAMPLES")?;
    let path = Path::new(&dir).join(name);
    path.exists().then_some(path)
}

/// Where renders go: `$ANICRAFT_VISUAL_OUT/anicraft-cli` when set, else a folder under the target directory (the tests
/// read the files back, so they need somewhere to write).
fn out_dir() -> PathBuf {
    let dir = std::env::var_os("ANICRAFT_VISUAL_OUT")
        .map_or_else(|| Path::new(env!("CARGO_TARGET_TMPDIR")).join("anicraft-cli"), |root| PathBuf::from(root).join("anicraft-cli"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Renders `frame` of `file` and returns the image and where it was written.
fn render(file: &Path, frame: u32, stem: &str) -> (common::Pixels, PathBuf) {
    let out = out_dir().join(format!("{stem}_f{frame}.png"));
    cli_ok(&["render", file.to_str().unwrap(), "--frame", &frame.to_string(), "-o", out.to_str().unwrap()]);
    (read_png(&out), out)
}

/// Pixels that differ from the corner pixel, i.e. things drawn on the background.
fn ink(img: &common::Pixels) -> usize {
    let bg = img.at(img.width - 1, img.height - 1);
    (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| img.at(x, y) != bg).count()
}

#[test]
fn unicorn_renders_the_first_and_a_middle_frame() {
    let Some(file) = sample("unicorn_4AE.fla") else { return };
    let (first, _) = render(&file, 1, "unicorn");
    assert_eq!((first.width, first.height), (1920, 1080));
    // The animation starts at frame 76: the first frame is the empty stage.
    assert_eq!(ink(&first), 0, "frame 1 is blank");
    let (middle, _) = render(&file, 1200, "unicorn");
    assert!(ink(&middle) > 20_000, "the unicorn is on stage in frame 1200");
    // The character stands in the left half of the stage.
    let left: usize = (0..middle.height).map(|y| (0..960).filter(|&x| middle.at(x, y) != [255, 255, 255, 255]).count()).sum();
    assert!(left > ink(&middle) / 2);
}

#[test]
fn derwood_renders_its_text() {
    let Some(file) = sample("derwood_green_choreo_short.fla") else { return };
    let (first, _) = render(&file, 1, "derwood");
    assert_eq!((first.width, first.height), (800, 600));
    let (middle, _) = render(&file, 700, "derwood");
    assert!(ink(&middle) > 500, "the words of the choreography are drawn");
    let (other, _) = render(&file, 40, "derwood");
    assert_ne!(middle.rgba, other.rgba, "the text changes over time");
}

#[test]
fn derwood_exports_video_with_its_soundtrack() {
    let Some(file) = sample("derwood_green_choreo_short.fla") else { return };
    let out = out_dir().join("derwood.mp4");
    cli_ok(&["export", file.to_str().unwrap(), "--format", "mp4", "--from", "1", "--to", "24", "--fps", "12", "-o", out.to_str().unwrap(), "-q"]);
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    // An AAC track is present: its sample entry is `mp4a`.
    assert!(bytes.windows(4).any(|w| w == b"mp4a"), "the sound layers were mixed in");
    assert!(bytes.windows(4).any(|w| w == b"avc1"));
}

#[test]
fn both_samples_convert_and_read_back_equal() {
    for name in ["derwood_green_choreo_short.fla", "unicorn_4AE.fla"] {
        let Some(file) = sample(name) else { continue };
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("copy.fla");
        cli_ok(&["convert", file.to_str().unwrap(), out.to_str().unwrap()]);
        let a = cli_ok(&["info", file.to_str().unwrap(), "--json"]);
        let b = cli_ok(&["info", out.to_str().unwrap(), "--json"]);
        let (a, b): (serde_json::Value, serde_json::Value) = (serde_json::from_str(&a).unwrap(), serde_json::from_str(&b).unwrap());
        assert_eq!(a["scenes"], b["scenes"], "{name}");
        assert_eq!(a["library"], b["library"], "{name}");
        assert_eq!(a["summary"]["keyframes"], b["summary"]["keyframes"], "{name}");
    }
}
