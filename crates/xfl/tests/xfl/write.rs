//! The writer: read -> write -> read gives the same model, output is deterministic and well formed, and hostile or
//! odd content never produces a broken document.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::assert_is_empty,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code"
)]

use std::io::Write;

use anicraft_xfl::diff::{DiffOptions, compare_archives, compare_xml};
use anicraft_xfl::{
    Loaded, ReadOptions, Sidecar, WriteOptions, encode_lossless_bitmap, read_bytes, read_with, sniff_bitmap, write_fla_bytes, write_to, write_xfl_folder,
};
use anim_archive::{ArchiveWriter, Container, ZipWriter};
use anim_core::{Affine, Rgba8};
use anim_display::Filter;
use anim_scene::{
    BitmapCompression, BitmapItem, ColorEffect, Document, Element, Frame, InstanceFilter, LibraryItem, MediaStore, SoundItem, SymbolInstance, SymbolKind,
    TextElement, TextKind, TextRun,
};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use proptest::prelude::*;

const DOM: &str = include_str!("../fixtures/basic/DOMDocument.xml");
const BALL: &str = include_str!("../fixtures/basic/LIBRARY/Ball.xml");
const WEIRD: &str = include_str!("../fixtures/basic/LIBRARY/shapes/&#060Weird&#062.xml");
const PUBLISH: &str = include_str!("../fixtures/basic/PublishSettings.xml");
const METADATA: &str = include_str!("../fixtures/basic/META-INF/metadata.xml");
const MARKER: &str = include_str!("../fixtures/basic/basic.xfl");

fn bitmap_dat() -> Vec<u8> {
    let argb: [u8; 32] = [255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128, 100, 100, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut out = vec![0x03, 0x05];
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    for v in [0u32, 80, 0, 40] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0, 1]);
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&argb).unwrap();
    let packed = z.finish().unwrap();
    out.extend_from_slice(&u16::try_from(packed.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&packed);
    out.extend_from_slice(&[0, 0]);
    out
}

fn fixture_zip(dom: &str) -> Vec<u8> {
    let mut writer = ZipWriter::in_memory();
    for (name, data) in [
        ("DOMDocument.xml", dom.as_bytes().to_vec()),
        ("LIBRARY/Ball.xml", BALL.as_bytes().to_vec()),
        ("LIBRARY/shapes/&#060Weird&#062.xml", WEIRD.as_bytes().to_vec()),
        ("PublishSettings.xml", PUBLISH.as_bytes().to_vec()),
        ("META-INF/metadata.xml", METADATA.as_bytes().to_vec()),
        ("basic.xfl", MARKER.as_bytes().to_vec()),
        ("bin/M 1 1700000001.dat", bitmap_dat()),
        ("bin/M 2 1700000002.dat", vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 9, 9, 9]),
        ("LIBRARY/click.wav", vec![7; 100]),
    ] {
        writer.add_file(name, &data).unwrap();
    }
    writer.finish().unwrap()
}

fn load_fixture() -> Loaded {
    read_bytes(fixture_zip(DOM), &ReadOptions::default()).unwrap()
}

fn write_bytes(loaded: &Loaded) -> Vec<u8> {
    write_fla_bytes(&loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default()).unwrap().0
}

#[test]
fn fixture_survives_write_and_read() {
    let first = load_fixture();
    let bytes = write_bytes(&first);
    let second = read_bytes(bytes, &ReadOptions::default()).unwrap();
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    assert_eq!(first.document, second.document);
    assert_eq!(first.media, second.media);
    // Sidecars come back: the marker, the metadata and the source sound file.
    let names = |l: &Loaded| {
        let mut n: Vec<String> = l.sidecars.iter().map(|s| s.name.clone()).filter(|n| n != "MobileSettings.xml").collect();
        n.sort();
        n
    };
    assert_eq!(names(&first), names(&second));
    // `MobileSettings.xml` is always written (an empty one when the document had none).
    assert!(second.sidecars.iter().any(|s| s.name == "MobileSettings.xml"));
    assert_eq!(first.sidecars.iter().find(|s| s.name == "LIBRARY/click.wav").map(|s| s.data.len()), Some(100));
}

#[test]
fn writing_twice_gives_identical_bytes() {
    let loaded = load_fixture();
    assert_eq!(write_bytes(&loaded), write_bytes(&loaded));
    // And writing what was read from our own output is a fixed point.
    let again = read_bytes(write_bytes(&loaded), &ReadOptions::default()).unwrap();
    assert_eq!(write_bytes(&loaded), write_bytes(&again));
}

#[test]
fn zip_layout_starts_with_a_stored_mimetype() {
    let loaded = load_fixture();
    let bytes = write_bytes(&loaded);
    // The first local header names `mimetype` and its data follows uncompressed.
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    assert_eq!(&bytes[30..38], b"mimetype");
    assert_eq!(&bytes[38..38 + 25], b"application/vnd.adobe.xfl");
    let container = Container::from_bytes(bytes).unwrap();
    assert!(container.warnings().is_empty(), "{:?}", container.warnings());
    for name in [
        "mimetype",
        "basic.xfl",
        "DOMDocument.xml",
        "PublishSettings.xml",
        "MobileSettings.xml",
        "META-INF/metadata.xml",
        "LIBRARY/Ball.xml",
        "bin/M 1 1700000001.dat",
    ] {
        assert!(container.exists(name), "missing {name}");
    }
    assert!(container.exists("LIBRARY/shapes/&#060Weird&#062.xml"));
    assert_eq!(container.read_text("mimetype").unwrap(), "application/vnd.adobe.xfl");
    assert_eq!(container.read_text("basic.xfl").unwrap().trim(), "PROXY-CS5");
}

#[test]
fn folder_output_reads_back_equal() {
    let first = load_fixture();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out");
    let report = write_xfl_folder(&target, &first.document, &first.media, &first.sidecars, &WriteOptions::default()).unwrap();
    assert_eq!(report.symbols, 2);
    assert_eq!(report.media_entries, 2);
    assert!(target.join("DOMDocument.xml").exists());
    assert!(target.join("basic.xfl").exists());
    // `<` in a symbol's file name is escaped the way Animate does.
    assert!(target.join("LIBRARY").join("shapes").join("&#060Weird&#062.xml").exists());
    let second = read_with(&target, &ReadOptions::default()).unwrap();
    assert_eq!(first.document, second.document);
    assert_eq!(first.media, second.media);
    // The `*.xfl` marker opens the folder too.
    let third = read_with(target.join("basic.xfl"), &ReadOptions::default()).unwrap();
    assert_eq!(first.document, third.document);
}

#[test]
fn write_path_picks_the_form_from_the_name() {
    let first = load_fixture();
    let dir = tempfile::tempdir().unwrap();
    let fla = dir.path().join("movie.fla");
    anicraft_xfl::write_path(&fla, &first.document, &first.media, &[], &WriteOptions::default()).unwrap();
    assert_eq!(read_with(&fla, &ReadOptions::default()).unwrap().document, first.document);
    // The marker is named after the file, and no temporary file is left behind.
    assert!(Container::open(&fla).unwrap().exists("movie.xfl"));
    assert!(!dir.path().join("movie.fla.tmp").exists());
    let marker = dir.path().join("sub").join("proj.xfl");
    anicraft_xfl::write_path(&marker, &first.document, &first.media, &[], &WriteOptions::default()).unwrap();
    assert!(marker.exists());
    assert_eq!(read_with(&marker, &ReadOptions::default()).unwrap().document, first.document);
}

#[test]
fn archive_comparison_of_the_fixture_is_clean_apart_from_what_the_reader_dropped() {
    let source = fixture_zip(DOM);
    let first = read_bytes(source.clone(), &ReadOptions::default()).unwrap();
    let written = write_bytes(&first);
    let diff = compare_archives(&Container::from_bytes(source).unwrap(), &Container::from_bytes(written).unwrap(), &DiffOptions::default());
    // The only entries the original has and the copy lacks are generated ones (`mimetype` is new in the copy).
    assert!(diff.only_in_first.is_empty(), "{:?}", diff.only_in_first);
    assert_eq!(diff.only_in_second, ["mimetype", "MobileSettings.xml"]);
    assert!(diff.bytes_differ.is_empty(), "{:?}", diff.bytes_differ);
    // Differences in XML are limited to content the reader cannot hold (see the known gaps of the reader): the
    // `Missing.xml` include and the hand-made quirks of the fixture.
    for d in &diff.document.differences {
        println!("{} [{:?}] {}", d.path, d.kind, d.detail);
    }
    assert!(diff.symbols.is_empty(), "{:?}", diff.symbols);
}

#[test]
fn imported_png_becomes_a_lossless_dat_and_new_names_are_generated() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 3, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255]).unwrap();
    }
    let mut store = MediaStore::new();
    let png_id = store.insert(png).unwrap();
    let sound_id = store.insert(vec![5; 40]).unwrap();
    let mut doc = Document::new(320.0, 240.0, 30.0);
    let mut bitmap = BitmapItem::new("pics/red", "id-1");
    bitmap.media = Some(png_id);
    bitmap.compression = BitmapCompression::Lossless;
    doc.library.items.push(LibraryItem::Bitmap(bitmap));
    let mut sound = SoundItem::new("beep", "id-2");
    sound.media = Some(sound_id);
    doc.library.items.push(LibraryItem::Sound(sound));
    doc.add_symbol("parts/a:b", SymbolKind::Graphic).unwrap();

    let (bytes, report) = write_fla_bytes(&doc, &store, &[], &WriteOptions::default()).unwrap();
    assert_eq!(report.media_entries, 2);
    let container = Container::from_bytes(bytes.clone()).unwrap();
    let names: Vec<&str> = container.files().map(|e| e.name.as_str()).collect();
    assert!(names.iter().any(|n| n.starts_with("bin/M 1 ") && std::path::Path::new(n).extension().is_some_and(|e| e.eq_ignore_ascii_case("dat"))), "{names:?}");
    assert!(names.contains(&"LIBRARY/parts/a&#058b.xml"), "{names:?}");

    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let item = back.document.library.find_bitmap("pics/red").unwrap();
    assert_eq!(item.size, Some([3.0, 2.0]));
    let blob = back.media.get(item.media.unwrap()).unwrap();
    assert_eq!(sniff_bitmap(blob), anicraft_xfl::BitmapFormat::Lossless);
    let image = anicraft_xfl::decode_bitmap(&back.media, item.media.unwrap()).unwrap();
    assert_eq!(&image.premultiplied_rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(back.document.library.find_sound("beep").map(|s| s.media.is_some()), Some(true));
    assert!(back.document.library.find_symbol("parts/a:b").is_some());
}

#[test]
fn photo_bitmaps_keep_their_jpeg_bytes() {
    let mut store = MediaStore::new();
    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];
    let id = store.insert(jpeg.clone()).unwrap();
    let mut doc = Document::default();
    let mut bitmap = BitmapItem::new("photo", "p");
    bitmap.media = Some(id);
    bitmap.compression = BitmapCompression::Photo;
    bitmap.quality = Some(80);
    bitmap.size = Some([10.0, 5.0]);
    doc.library.items.push(LibraryItem::Bitmap(bitmap));
    let (bytes, _) = write_fla_bytes(&doc, &store, &[], &WriteOptions::default()).unwrap();
    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    let item = back.document.library.find_bitmap("photo").unwrap();
    assert_eq!(item.compression, BitmapCompression::Photo);
    assert_eq!(item.quality, Some(80));
    assert_eq!(back.media.get(item.media.unwrap()), Some(&jpeg[..]));
}

#[test]
fn text_filters_colours_and_scripts_round_trip() {
    let mut doc = Document::default();
    let mut instance = SymbolInstance::new("lib/thing", Affine::new([1.0, 0.5, -0.5, 1.0, 12.5, -3.25]));
    instance.set_color_effect(ColorEffect::Tint { color: Rgba8::opaque(1, 2, 3), amount: 0.25 });
    instance.filters.push(InstanceFilter::mapped(Filter::Blur { x: 4.0, y: 2.0, quality: 3 }));
    instance.filters.push(InstanceFilter::mapped(Filter::DropShadow {
        color: Rgba8::new(10, 20, 30, 128),
        angle_deg: 30.0,
        distance: 7.5,
        blur_x: 1.0,
        blur_y: 2.0,
        strength: 0.5,
        inner: true,
        knockout: false,
        hide_object: true,
    }));
    instance.name = Some("quote\"d & <name>".to_owned());
    let mut text = TextElement::new(
        TextKind::Input,
        Affine::IDENTITY,
        100.0,
        20.0,
        vec![TextRun::new("שלום עולם \u{627}\u{644}\u{633}\u{644}\u{627}\u{645}\r\nline & <two>", "Arial", 18.5)],
    );
    text.instance_name = Some("field".to_owned());
    let mut frame = Frame::new(0, 4, vec![Element::SymbolInstance(Box::new(instance)), Element::Text(Box::new(text))]);
    frame.actionscript = Some("trace(\"a]]>b\"); // <tag> & more\r\nstop();".to_owned());
    frame.label = Some("start".to_owned());
    doc.scenes.first_mut().unwrap().layers.first_mut().unwrap().frames = vec![frame];

    let (bytes, report) = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let a = doc.scenes.first().unwrap().layers.first().unwrap().frames.first().unwrap();
    let b = back.document.scenes.first().unwrap().layers.first().unwrap().frames.first().unwrap();
    assert_eq!(a.elements.len(), b.elements.len());
    // A carriage return in the script is escaped, so it survives (a CDATA section would turn it into a line feed).
    assert_eq!(b.actionscript.as_deref(), Some("trace(\"a]]>b\"); // <tag> & more\r\nstop();"));
    assert_eq!(a.elements, b.elements);
    assert_eq!(a.label, b.label);
}

#[test]
fn component_instances_round_trip_with_their_parameters() {
    let mut doc = Document::new(100.0, 100.0, 24.0);
    let mut instance = SymbolInstance::new("Button", Affine::IDENTITY);
    instance.name = Some("go_btn".to_owned());
    instance.component = Some(anim_scene::ComponentInstance::from_params(&[("label", "Go & \"play\"", "String"), ("enabled", "true", "Boolean")]));
    let frame = Frame::new(0, 4, vec![Element::SymbolInstance(Box::new(instance))]);
    doc.scenes.first_mut().unwrap().layers.first_mut().unwrap().frames = vec![frame];
    let (bytes, _) = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap();
    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    let Element::SymbolInstance(i) = &back.document.scenes[0].layers[0].frames[0].elements[0] else { panic!("not an instance") };
    let c = i.component.as_ref().expect("component survives");
    assert_eq!(c.param("label").as_deref(), Some("Go & \"play\""));
    assert_eq!(c.param("enabled").as_deref(), Some("true"));
}

#[test]
fn unusable_kept_xml_is_left_out_with_a_warning() {
    let mut doc = Document::default();
    doc.extra.children.push("<ok a=\"1\"/>".to_owned());
    doc.extra.children.push("<broken".to_owned());
    doc.extra.children.push("<a/><b/>".to_owned());
    doc.extra.attrs.push(("bad name".to_owned(), "x".to_owned()));
    doc.extra.attrs.push(("fine".to_owned(), "line\nbreak\ttab \u{1} end".to_owned()));
    doc.extra.attrs.push(("fine".to_owned(), "second value is dropped".to_owned()));
    let (bytes, report) = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap();
    assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    assert_eq!(back.document.extra.attr("fine"), Some("line\nbreak\ttab  end"));
    assert!(back.document.extra.attr("bad name").is_none());
    assert_eq!(back.document.extra.children, ["<ok a=\"1\"/>"]);
}

#[test]
fn duplicate_and_unsafe_names_are_repaired() {
    let mut doc = Document::default();
    for (name, href) in [("a", Some("same.xml")), ("b", Some("SAME.xml")), ("c", Some("../escape.xml")), ("d", None)] {
        doc.add_symbol(name, SymbolKind::MovieClip).unwrap();
        doc.library.find_symbol_mut(name).unwrap().href = href.map(str::to_owned);
    }
    let (bytes, report) = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap();
    assert_eq!(report.symbols, 4);
    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(back.document.library.symbols().count(), 4);
    assert_eq!(doc.library.symbols().count(), 4);
}

#[test]
fn semantic_diff_ignores_formatting() {
    let loaded = load_fixture();
    let (dom, warnings) = anicraft_xfl::write::document_xml(&loaded.document, &loaded.media);
    assert!(warnings.is_empty(), "{warnings:?}");
    // Everything the document held comes back with the same values. What differs is the dropped `Missing.xml`
    // include and the order of a few children of the hand-written fixture, which is not Animate's order.
    let diff = compare_xml(DOM, &dom, &DiffOptions::default());
    for d in &diff.differences {
        println!("{} [{:?}] {}", d.path, d.kind, d.detail);
    }
    for key in diff.by_key.keys() {
        assert!(key.starts_with("Tag:") || key.starts_with("ChildCount:"), "{:?}", diff.top(10));
    }
    assert!(diff.total <= 8, "{:?}", diff.top(10));
}

#[test]
fn encoder_blobs_are_what_animate_style_readers_accept() {
    let image = anim_display::Image::new(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255]).unwrap();
    let blob = encode_lossless_bitmap(&image).unwrap();
    assert_eq!(&blob[..2], &[3, 5]);
    assert_eq!(&blob[blob.len() - 2..], &[0, 0]);
}

#[test]
fn write_to_reports_duplicate_sidecars_instead_of_failing() {
    let loaded = load_fixture();
    let mut sidecars = loaded.sidecars.clone();
    sidecars.push(Sidecar { name: "extra.txt".to_owned(), data: b"1".to_vec() });
    sidecars.push(Sidecar { name: "EXTRA.txt".to_owned(), data: b"2".to_vec() });
    sidecars.push(Sidecar { name: "../evil".to_owned(), data: b"3".to_vec() });
    let mut writer = ZipWriter::in_memory();
    let report = write_to(&mut writer, &loaded.document, &loaded.media, &sidecars, &WriteOptions::default()).unwrap();
    assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
    let container = Container::from_bytes(writer.finish().unwrap()).unwrap();
    assert!(container.exists("extra.txt"));
}

#[test]
fn symdepend_cache_is_only_kept_on_request() {
    let loaded = load_fixture();
    let mut sidecars = loaded.sidecars.clone();
    sidecars.push(Sidecar { name: "bin/SymDepend.cache".to_owned(), data: vec![1, 2, 3] });
    let plain = write_fla_bytes(&loaded.document, &loaded.media, &sidecars, &WriteOptions::default()).unwrap().0;
    assert!(!Container::from_bytes(plain).unwrap().exists("bin/SymDepend.cache"));
    let keep = WriteOptions { keep_caches: true, ..WriteOptions::default() };
    let kept = write_fla_bytes(&loaded.document, &loaded.media, &sidecars, &keep).unwrap().0;
    assert!(Container::from_bytes(kept).unwrap().exists("bin/SymDepend.cache"));
}

fn mutate(text: &str, at: usize, junk: &[u8], delete: usize) -> String {
    let bytes = text.as_bytes();
    let at = at % bytes.len().max(1);
    let end = (at + delete).min(bytes.len());
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(junk);
    out.extend_from_slice(&bytes[end..]);
    String::from_utf8_lossy(&out).into_owned()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Whatever the reader makes of a damaged document, the writer produces a file that reads back without error,
    /// and writing that again is a fixed point of the model.
    #[test]
    fn damaged_documents_write_to_readable_output(at in 0usize..40_000, junk in prop::collection::vec(any::<u8>(), 0..24), delete in 0usize..64) {
        let dom = mutate(DOM, at, &junk, delete);
        if let Ok(first) = read_bytes(fixture_zip(&dom), &ReadOptions::default()) {
            let (bytes, _) = write_fla_bytes(&first.document, &first.media, &first.sidecars, &WriteOptions::default()).unwrap();
            let second = read_bytes(bytes, &ReadOptions::default()).unwrap();
            let (again, _) = write_fla_bytes(&second.document, &second.media, &second.sidecars, &WriteOptions::default()).unwrap();
            let third = read_bytes(again, &ReadOptions::default()).unwrap();
            prop_assert_eq!(second.document, third.document);
        }
    }

    /// Text and attribute values with any XML-legal characters survive.
    #[test]
    fn arbitrary_text_survives(text in "[^\u{0}-\u{8}\u{b}\u{c}\u{e}-\u{1f}\u{fffe}\u{ffff}]{0,40}", value in "[^\u{0}-\u{8}\u{b}\u{c}\u{e}-\u{1f}\u{fffe}\u{ffff}]{0,40}") {
        let mut doc = Document::default();
        doc.extra.attrs.push(("custom".to_owned(), value.clone()));
        let run = TextRun::new(text.clone(), "Arial", 12.0);
        let mut field = TextElement::new(TextKind::Static, Affine::IDENTITY, 10.0, 10.0, vec![run]);
        field.extra.attrs.push(("note".to_owned(), value.clone()));
        let frame = Frame::new(0, 1, vec![Element::Text(Box::new(field))]);
        doc.scenes.first_mut().unwrap().layers.first_mut().unwrap().frames = vec![frame];
        let (bytes, _) = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap();
        let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
        prop_assert_eq!(back.document.extra.attr("custom"), Some(value.as_str()));
        let read_frame = back.document.scenes.first().unwrap().layers.first().unwrap().frames.first().unwrap();
        let Some(Element::Text(read_field)) = read_frame.elements.first() else { panic!("no text") };
        prop_assert_eq!(read_field.plain_text(), text);
        prop_assert_eq!(read_field.extra.attr("note"), Some(value.as_str()));
    }
}
