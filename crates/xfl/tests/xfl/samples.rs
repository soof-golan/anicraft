//! Tests against the Adobe-authored sample files. They read `ANICRAFT_SAMPLES` and pass trivially when it is unset.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::assert_is_empty,
    clippy::chunks_exact_to_as_chunks,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code"
)]

use std::path::PathBuf;

use anicraft_xfl::{Loaded, ReadOptions, decode_bitmap, read_with, sound_payload};
use anim_scene::{Document, Element, LibraryItem, Timeline};

fn sample(name: &str) -> Option<PathBuf> {
    let root = std::env::var_os("ANICRAFT_SAMPLES")?;
    let path = PathBuf::from(root).join(name);
    path.exists().then_some(path)
}

fn all_timelines(doc: &Document) -> Vec<&Timeline> {
    let mut out: Vec<&Timeline> = doc.scenes.iter().collect();
    out.extend(doc.library.symbols().map(|s| &s.timeline));
    out
}

/// Visits every element of every timeline, groups included.
fn for_each_element<'a>(doc: &'a Document, f: &mut impl FnMut(&'a Element)) {
    for timeline in all_timelines(doc) {
        for layer in &timeline.layers {
            for frame in &layer.frames {
                for element in &frame.elements {
                    element.walk(f);
                }
            }
        }
    }
}

/// Parsing succeeded without leaving anything unexplained: no warnings, and the only child elements in `extra` are
/// the document sections the model has no type for.
fn assert_clean(loaded: &Loaded) {
    assert!(loaded.warnings.is_empty(), "{:#?}", loaded.warnings);
    let allowed = ["DOMDocument/PrinterSettings", "DOMDocument/publishHistory", "DOMDocument/scripts", "DOMDocument/swatchLists"];
    for key in loaded.unknowns.children.keys() {
        assert!(allowed.contains(&key.as_str()), "unexpected unknown element {key}: {:?}", loaded.unknowns.children);
    }
}

fn assert_every_edge_parses(loaded: &Loaded) {
    let mut shapes = 0usize;
    let mut edges = 0usize;
    let mut segments = 0usize;
    for_each_element(&loaded.document, &mut |e| {
        if let Element::Shape(s) = e {
            shapes += 1;
            for edge in &s.shape.edges {
                edges += 1;
                segments += edge.segments.len();
                if edge.original_edges_text().is_some_and(|t| !t.trim().is_empty()) {
                    assert!(!edge.segments.is_empty(), "edge with text but no segments");
                }
                assert!(edge.is_unedited(), "an untouched edge must write back verbatim");
            }
        }
    });
    assert!(shapes > 0 && edges > 0 && segments > 0);
    assert!(loaded.warnings.iter().all(|w| !w.message.contains("DOMShape")), "{:?}", loaded.warnings);
}

#[test]
fn unicorn_loads_with_the_expected_library_and_timelines() {
    let Some(path) = sample("unicorn_4AE.fla") else {
        return;
    };
    let loaded = read_with(&path, &ReadOptions::default()).unwrap();
    assert_clean(&loaded);
    let doc = &loaded.document;
    assert_eq!((doc.width, doc.height, doc.frame_rate), (1920.0, 1080.0, 30.0));
    assert_eq!(doc.current_timeline, 0);
    let stats = doc.stats();
    assert_eq!((stats.scenes, stats.symbols, stats.bitmaps, stats.sounds, stats.videos, stats.folders, stats.fonts), (1, 45, 1, 1, 1, 2, 0), "{stats:?}");
    assert_eq!((stats.layers, stats.keyframes, stats.total_frames), (87, 419, 3009), "{stats:?}");
    let scene = &doc.scenes[0];
    assert_eq!(scene.name, "Scene 1");
    assert_eq!(scene.current_frame, 160);
    assert_eq!(scene.frame_count(), 3009);
    assert_eq!(doc.library.items.len(), 2 + 3 + 45);
    // Library names keep Animate's `&#NNN` escapes and every Include resolves.
    assert!(doc.library.find_symbol("flash0.ai_2/Isolation Mode/&#060Clip Group&#062").is_some());
    assert!(doc.library.symbols().all(|s| s.href.is_some() && !s.item_id.is_empty()));
    assert_every_edge_parses(&loaded);
    // Every instance points at a library symbol.
    let mut instances = 0usize;
    for_each_element(doc, &mut |e| {
        if let Element::SymbolInstance(i) = e {
            instances += 1;
            assert!(doc.library.find_symbol(&i.library_item_name).is_some(), "dangling {}", i.library_item_name);
        }
    });
    assert!(instances > 300);
    // Tweens, eases, filters and guide layers of the real file all arrive.
    let mut tweens = 0usize;
    let mut custom_eases = 0usize;
    let mut filters = 0usize;
    for timeline in all_timelines(doc) {
        for layer in &timeline.layers {
            for frame in &layer.frames {
                tweens += usize::from(frame.is_tweened());
                custom_eases += frame.eases.iter().filter(|e| matches!(e.curve, anim_scene::EaseCurve::Custom(_))).count();
                for e in &frame.elements {
                    if let Some(i) = e.as_symbol_instance() {
                        filters += i.filters.len();
                    }
                }
            }
        }
    }
    assert_eq!((tweens, custom_eases, filters), (77, 4, 5));
    assert_eq!(all_timelines(doc).iter().flat_map(|t| &t.layers).filter(|l| l.kind == anim_scene::LayerKind::Guide).count(), 2);
}

#[test]
fn unicorn_media_decodes() {
    let Some(path) = sample("unicorn_4AE.fla") else {
        return;
    };
    let loaded = read_with(&path, &ReadOptions::default()).unwrap();
    let lib = &loaded.document.library;
    let bitmap = lib.bitmaps().next().unwrap();
    assert_eq!(bitmap.name, "Bitmap 3");
    assert_eq!(bitmap.size, Some([959.0, 543.0]));
    let image = decode_bitmap(&loaded.media, bitmap.media.unwrap()).unwrap();
    assert_eq!((image.width, image.height), (959, 543));
    assert!(image.premultiplied_rgba.chunks_exact(4).all(|p| p[3] == 255));
    // The photo has real content: not a single colour.
    let first = &image.premultiplied_rgba[..4];
    assert!(image.premultiplied_rgba.chunks_exact(4).any(|p| p != first));
    let sound = lib.sounds().next().unwrap();
    assert_eq!(sound.sample_count, Some(4_424_395));
    let payload = sound_payload(&loaded.media, sound).unwrap();
    assert_eq!(payload.len(), 4_424_395 * 4, "16 bit stereo PCM");
    let Some(LibraryItem::Video(video)) = lib.items.iter().find(|i| matches!(i, LibraryItem::Video(_))) else {
        panic!("video item");
    };
    assert_eq!((video.width, video.height, video.fps), (Some(1920.0), Some(1080.0), Some(30.0)));
    assert!(loaded.media.get(video.media.unwrap()).is_some_and(|b| b.len() > 1_000_000));
}

#[test]
fn derwood_loads_text_and_sounds() {
    let Some(path) = sample("derwood_green_choreo_short.fla") else {
        return;
    };
    let loaded = read_with(&path, &ReadOptions::default()).unwrap();
    assert_clean(&loaded);
    let doc = &loaded.document;
    assert_eq!((doc.width, doc.height, doc.frame_rate), (800.0, 600.0, 4.0));
    let stats = doc.stats();
    assert_eq!((stats.layers, stats.keyframes, stats.total_frames, stats.sounds, stats.symbols), (3, 29, 1451, 2, 0), "{stats:?}");
    let mut texts = Vec::new();
    for_each_element(doc, &mut |e| {
        if let Element::Text(t) = e {
            texts.push(t.plain_text());
        }
    });
    assert_eq!(texts.len(), 24);
    assert_eq!(texts[0], "slow");
    assert!(texts.iter().all(|t| !t.is_empty()));
    for sound in doc.library.sounds() {
        let payload = sound_payload(&loaded.media, sound).unwrap();
        let declared: usize = sound.extra.attr("dataLength").unwrap().parse().unwrap();
        assert_eq!(payload.len(), declared);
        // Compressed (MP3) data starts with an ID3 tag or a frame sync.
        assert!(payload.starts_with(b"ID3") || payload.first() == Some(&0xFF), "{:?}", &payload[..4]);
    }
    let with_sound: usize = doc.scenes[0].layers.iter().flat_map(|l| &l.frames).filter(|f| f.sound.is_some()).count();
    assert_eq!(with_sound, 1, "one of the two sound items is placed on the timeline");
    assert!(doc.validate().is_empty());
    assert!(loaded.sidecars.iter().any(|s| s.name == "META-INF/metadata.xml"));
    assert!(doc.publish_settings_xml.is_some());
}

/// The survey the scripting support is scoped from: how many frame scripts and component instances the samples
/// hold. The numbers are printed with `--nocapture`.
#[test]
fn samples_script_and_component_survey() {
    for name in ["unicorn_4AE.fla", "derwood_green_choreo_short.fla"] {
        let Some(path) = sample(name) else {
            continue;
        };
        let loaded = read_with(&path, &ReadOptions { load_media: false, ..ReadOptions::default() }).unwrap();
        let mut scripts = Vec::new();
        let mut components = 0usize;
        for timeline in all_timelines(&loaded.document) {
            for layer in &timeline.layers {
                for frame in &layer.frames {
                    if let Some(s) = &frame.actionscript {
                        scripts.push(s.clone());
                    }
                }
            }
        }
        for_each_element(&loaded.document, &mut |e| {
            if let Element::SymbolInstance(i) = e {
                components += usize::from(i.component.is_some());
            }
        });
        println!("survey {name}: {} frame scripts, {components} component instances, first scripts {:?}", scripts.len(), scripts.iter().take(5).collect::<Vec<_>>());
    }
}

#[test]
fn reading_without_media_is_faster_and_leaves_ids_empty() {
    let Some(path) = sample("unicorn_4AE.fla") else {
        return;
    };
    let loaded = read_with(&path, &ReadOptions { load_media: false, ..ReadOptions::default() }).unwrap();
    assert!(loaded.media.is_empty());
    assert!(loaded.document.library.items.iter().all(|i| i.media().is_none()));
    assert_eq!(loaded.document.stats().symbols, 45);
}
