//! Hostile input: truncated, mutated and generated documents must produce an error or a document, never a panic.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::expect_used,
    clippy::format_push_string,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::string_slice,
    clippy::unwrap_used,
    reason = "test code"
)]

use anicraft_xfl::{ReadOptions, decode_bitmap_bytes, read_bytes};
use anim_archive::{ArchiveWriter, ZipWriter};
use proptest::prelude::*;

const DOM: &str = include_str!("../fixtures/basic/DOMDocument.xml");
const BALL: &str = include_str!("../fixtures/basic/LIBRARY/Ball.xml");

fn zip_with(dom: &[u8], symbol: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::in_memory();
    writer.add_file("DOMDocument.xml", dom).unwrap();
    writer.add_file("LIBRARY/Ball.xml", symbol).unwrap();
    writer.add_file("LIBRARY/shapes/&#060Weird&#062.xml", b"<DOMSymbolItem name=\"x\"/>").unwrap();
    writer.add_file("bin/M 1 1700000001.dat", &[3, 5, 16, 0, 4, 0, 2, 0]).unwrap();
    writer.finish().unwrap()
}

fn mutate(text: &str, at: usize, junk: &[u8], delete: usize) -> Vec<u8> {
    let bytes = text.as_bytes();
    let at = at % bytes.len().max(1);
    let end = (at + delete).min(bytes.len());
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(junk);
    out.extend_from_slice(&bytes[end..]);
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn mutated_documents_never_panic(at in 0usize..40_000, junk in prop::collection::vec(any::<u8>(), 0..24), delete in 0usize..64) {
        let dom = mutate(DOM, at, &junk, delete);
        let _ = read_bytes(zip_with(&dom, BALL.as_bytes()), &ReadOptions::default());
    }

    #[test]
    fn mutated_symbol_files_never_panic(at in 0usize..4_000, junk in prop::collection::vec(any::<u8>(), 0..24), delete in 0usize..64) {
        let symbol = mutate(BALL, at, &junk, delete);
        let _ = read_bytes(zip_with(DOM.as_bytes(), &symbol), &ReadOptions::default());
    }

    #[test]
    fn truncated_documents_never_panic(cut in 0usize..40_000) {
        let bytes = DOM.as_bytes();
        let dom = &bytes[..cut.min(bytes.len())];
        let _ = read_bytes(zip_with(dom, BALL.as_bytes()), &ReadOptions::default());
    }

    #[test]
    fn random_bytes_as_a_document_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = read_bytes(zip_with(&bytes, &bytes), &ReadOptions::default());
    }

    #[test]
    fn numbers_at_the_edges_are_survivable(value in prop::sample::select(vec!["4294967295", "-1", "1e308", "NaN", "inf", "0", "99999999999999999999", "-0.0", "", "1,5"])) {
        // Replace every numeric attribute value in the fixture with the hostile one.
        let mut out = String::new();
        let mut rest = DOM;
        while let Some(i) = rest.find("=\"") {
            out.push_str(&rest[..i + 2]);
            rest = &rest[i + 2..];
            let Some(j) = rest.find('"') else { break };
            let (v, tail) = rest.split_at(j);
            let numeric = v.parse::<f64>().is_ok();
            out.push_str(if numeric { value } else { v });
            rest = tail;
        }
        out.push_str(rest);
        let _ = read_bytes(zip_with(out.as_bytes(), BALL.as_bytes()), &ReadOptions::default());
    }

    #[test]
    fn random_bitmap_blobs_never_panic(tail in prop::collection::vec(any::<u8>(), 0..600), width in 0u16..64, height in 0u16..64) {
        let mut blob = vec![0x03, 0x05];
        blob.extend_from_slice(&(width.saturating_mul(4)).to_le_bytes());
        blob.extend_from_slice(&width.to_le_bytes());
        blob.extend_from_slice(&height.to_le_bytes());
        blob.extend_from_slice(&[0; 18]);
        blob.extend_from_slice(&tail);
        let _ = decode_bitmap_bytes(&blob);
        let _ = decode_bitmap_bytes(&tail);
    }
}

#[test]
fn deep_nesting_is_an_error_or_bounded_not_a_stack_overflow() {
    for depth in [100usize, 1_000, 100_000] {
        let mut dom = String::from("<DOMDocument><timelines><DOMTimeline><layers><DOMLayer><frames><DOMFrame><elements>");
        for _ in 0..depth {
            dom.push_str("<DOMGroup><members>");
        }
        for _ in 0..depth {
            dom.push_str("</members></DOMGroup>");
        }
        dom.push_str("</elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>");
        let result = read_bytes(zip_with(dom.as_bytes(), BALL.as_bytes()), &ReadOptions::default());
        if depth == 100_000 {
            assert!(matches!(result, Err(anicraft_xfl::XflError::TooDeep(_))));
        } else if let Ok(loaded) = result {
            let mut count = 0usize;
            for e in &loaded.document.scenes[0].layers[0].frames[0].elements {
                e.walk(&mut |_| count += 1);
            }
            assert!(count <= anim_scene::MAX_GROUP_DEPTH + 2, "{count}");
        }
    }
}

#[test]
fn a_million_frames_and_layers_are_capped() {
    let mut dom = String::from("<DOMDocument><timelines><DOMTimeline><layers><DOMLayer><frames>");
    for i in 0..150_000 {
        dom.push_str(&format!("<DOMFrame index=\"{i}\"/>"));
    }
    dom.push_str("</frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>");
    let loaded = read_bytes(zip_with(dom.as_bytes(), BALL.as_bytes()), &ReadOptions::default()).unwrap();
    assert_eq!(loaded.document.scenes[0].layers[0].frames.len(), anicraft_xfl::timeline::MAX_FRAMES);
    assert!(loaded.warnings.iter().any(|w| w.message.contains("too many keyframes")));
}

#[test]
fn a_dtd_is_refused_not_expanded() {
    let dom = r#"<!DOCTYPE d [<!ENTITY a "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;">]><DOMDocument width="&b;"/>"#;
    assert!(read_bytes(zip_with(dom.as_bytes(), BALL.as_bytes()), &ReadOptions::default()).is_err());
}
