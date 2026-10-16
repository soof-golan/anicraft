//! Zipped FLA quirks seen in files written by Animate, built byte by byte so each one is isolated: a central
//! directory size that claims more than there is, entry names that are not UTF-8, and entity-escaped names.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code"
)]

use anicraft_xfl::{ReadOptions, read_bytes};

const DOM: &str = r##"<DOMDocument xmlns="http://ns.adobe.com/xfl/2008/" width="100" height="100" frameRate="12" backgroundColor="#FFFFFF" xflVersion="2.9">
  <symbols><Include href="Ball.xml" loadImmediate="false"/><Include href="a&amp;#060b&amp;#062.xml" loadImmediate="false"/></symbols>
  <timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="L"><frames><DOMFrame index="0" duration="1"><elements>
    <DOMSymbolInstance libraryItemName="Ball"><matrix><Matrix tx="10" ty="10"/></matrix></DOMSymbolInstance>
  </elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines>
</DOMDocument>"##;

const BALL: &str = r##"<DOMSymbolItem xmlns="http://ns.adobe.com/xfl/2008/" name="Ball" symbolType="graphic"><timeline><DOMTimeline name="Ball"><layers><DOMLayer name="L"><frames><DOMFrame index="0" duration="1"><elements>
  <DOMShape><fills><FillStyle index="1"><SolidColor color="#FF0000"/></FillStyle></fills><edges><Edge fillStyle1="1" edges="!0 0|400 0|400 400|0 400|0 0"/></edges></DOMShape>
</elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timeline></DOMSymbolItem>"##;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A stored zip. `cd_size_extra` is added to the size the end record claims for the central directory.
fn zip(entries: &[(&[u8], &[u8])], cd_size_extra: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let (len, nlen) = (data.len() as u32, name.len() as u16);
        out.extend_from_slice(&[0x50, 0x4B, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        central.extend_from_slice(&[0x50, 0x4B, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4B, 5, 6, 0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd_len + cd_size_extra).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn entries(extra_name: &[u8]) -> Vec<(&[u8], &[u8])> {
    vec![
        (b"DOMDocument.xml".as_slice(), DOM.as_bytes()),
        (b"LIBRARY/Ball.xml".as_slice(), BALL.as_bytes()),
        (b"LIBRARY/a&#060b&#062.xml".as_slice(), b"<DOMSymbolItem name=\"ab\"/>".as_slice()),
        (extra_name, b"opaque".as_slice()),
    ]
}

fn symbols(bytes: Vec<u8>) -> Vec<String> {
    let loaded = read_bytes(bytes, &ReadOptions::default()).expect("the archive reads");
    let mut names: Vec<String> = loaded.document.library.symbols().map(|s| s.name.clone()).collect();
    names.sort();
    names
}

#[test]
fn a_clean_archive_reads() {
    assert_eq!(symbols(zip(&entries(b"bin/M 1 1.dat"), 0)), ["Ball", "ab"]);
}

#[test]
fn a_central_directory_size_that_claims_too_much_is_tolerated() {
    for extra in [1, 100, 4096, 70_000] {
        assert_eq!(symbols(zip(&entries(b"bin/M 1 1.dat"), extra)), ["Ball", "ab"], "claimed +{extra}");
    }
}

#[test]
fn a_name_that_is_not_utf8_does_not_lose_the_rest() {
    // 0x82 is e-acute in code page 437, which older archivers write without the UTF-8 flag.
    assert_eq!(symbols(zip(&entries(b"bin/caf\x82.dat"), 0)), ["Ball", "ab"]);
    assert_eq!(symbols(zip(&entries(b"bin/caf\x82.dat"), 4096)), ["Ball", "ab"]);
}
