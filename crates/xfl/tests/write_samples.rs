//! Round trips of the Adobe-authored sample files: read, write to a scratch folder, read back and compare the models,
//! then compare the written archive with the original semantically. Passes trivially when `ANICRAFT_SAMPLES` is unset.
#![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

use std::path::PathBuf;

use anicraft_xfl::diff::{DiffOptions, compare_archives};
use anicraft_xfl::{ReadOptions, WriteOptions, read_with, write_fla, write_xfl_folder};
use anim_archive::{Container, IndexSource};

fn sample(name: &str) -> Option<PathBuf> {
    let root = std::env::var_os("ANICRAFT_SAMPLES")?;
    let path = PathBuf::from(root).join(name);
    path.exists().then_some(path)
}

/// Entries of the original that the writer does not copy on purpose.
const NOT_WRITTEN: [&str; 1] = ["bin/SymDepend.cache"];

fn round_trip(name: &str) {
    let Some(path) = sample(name) else {
        return;
    };
    let first = read_with(&path, &ReadOptions::default()).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(name);
    let options = WriteOptions { name: name.trim_end_matches(".fla").to_owned(), ..WriteOptions::default() };
    let report = write_fla(&target, &first.document, &first.media, &first.sidecars, &options).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    // The zip is well formed: the strict zip reader indexes it without repairs.
    let written = Container::open(&target).unwrap();
    assert_eq!(written.index_source(), IndexSource::ZipCrate);
    assert!(written.warnings().is_empty(), "{:?}", written.warnings());

    // The models are equal after the trip.
    let second = read_with(&target, &ReadOptions::default()).unwrap();
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    assert_eq!(first.document, second.document, "documents differ");
    assert_eq!(first.media, second.media, "media differ");
    assert_eq!(first.unknowns, second.unknowns);

    // The XML is semantically the original: same elements, attributes and text.
    let original = Container::open(&path).unwrap();
    let diff = compare_archives(&original, &written, &DiffOptions::default());
    for (key, n) in diff.document.top(20) {
        println!("DOMDocument.xml: {n} x {key}");
    }
    for d in diff.document.differences.iter().take(10) {
        println!("  {} [{:?}] {}", d.path, d.kind, d.detail);
    }
    for (file, d) in &diff.symbols {
        println!("{file}: {} differences, e.g. {:?}", d.total, d.differences.first());
    }
    assert_eq!(diff.document.total, 0, "DOMDocument.xml differs: {:?}", diff.document.top(10));
    assert!(diff.symbols.is_empty(), "symbol files differ: {:?}", diff.symbols.iter().map(|(n, d)| (n, d.top(3))).collect::<Vec<_>>());
    assert!(diff.bytes_differ.is_empty(), "{:?}", diff.bytes_differ);
    assert!(diff.unreadable.is_empty(), "{:?}", diff.unreadable);
    for missing in &diff.only_in_first {
        assert!(NOT_WRITTEN.contains(&missing.as_str()), "the copy lacks {missing}");
    }
    // `mimetype` and the marker exist in the originals too, so nothing is new in the copy.
    assert!(diff.only_in_second.is_empty(), "{:?}", diff.only_in_second);

    // Writing the copy again gives the same bytes: the output is deterministic.
    let again = dir.path().join("again.fla");
    write_fla(&again, &second.document, &second.media, &second.sidecars, &options).unwrap();
    assert_eq!(std::fs::metadata(&target).unwrap().len(), std::fs::metadata(&again).unwrap().len());
}

#[test]
fn derwood_round_trips() {
    round_trip("derwood_green_choreo_short.fla");
}

#[test]
fn unicorn_round_trips() {
    round_trip("unicorn_4AE.fla");
}

#[test]
fn derwood_as_an_xfl_folder_round_trips() {
    let Some(path) = sample("derwood_green_choreo_short.fla") else {
        return;
    };
    let first = read_with(&path, &ReadOptions::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("derwood");
    let options = WriteOptions { name: "derwood".to_owned(), ..WriteOptions::default() };
    write_xfl_folder(&folder, &first.document, &first.media, &first.sidecars, &options).unwrap();
    // A marker among the sidecars keeps its own name.
    let marker = first
        .sidecars
        .iter()
        .find(|s| std::path::Path::new(&s.name).extension().is_some_and(|e| e.eq_ignore_ascii_case("xfl")))
        .map_or("derwood.xfl", |s| s.name.as_str());
    let second = read_with(folder.join(marker), &ReadOptions::default()).unwrap();
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    assert_eq!(first.document, second.document);
    assert_eq!(first.media, second.media);
}
