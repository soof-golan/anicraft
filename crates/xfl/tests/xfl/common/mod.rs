//! The hand-written fixture in `tests/fixtures/basic` and the media that go with it, shared by the read and write
//! tests.
#![allow(clippy::unwrap_used, reason = "test code")]

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

pub const DOM: &str = include_str!("../../fixtures/basic/DOMDocument.xml");
pub const BALL: &str = include_str!("../../fixtures/basic/LIBRARY/Ball.xml");
pub const WEIRD: &str = include_str!("../../fixtures/basic/LIBRARY/shapes/&#060Weird&#062.xml");
pub const PUBLISH: &str = include_str!("../../fixtures/basic/PublishSettings.xml");
pub const METADATA: &str = include_str!("../../fixtures/basic/META-INF/metadata.xml");
pub const MARKER: &str = include_str!("../../fixtures/basic/basic.xfl");

/// A 4 x 2 lossless bitmap: row 0 opaque red, green, blue, white; row 1 half transparent grey, then three clear pixels.
pub fn bitmap_dat() -> Vec<u8> {
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

/// Eight bytes of sound followed by five bytes of preview cache.
pub fn sound_dat() -> Vec<u8> {
    vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 9, 9, 9]
}

/// The files of the fixture archive, with `dom` as its document.
pub fn entries(dom: &str) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("DOMDocument.xml", dom.as_bytes().to_vec()),
        ("LIBRARY/Ball.xml", BALL.as_bytes().to_vec()),
        ("LIBRARY/shapes/&#060Weird&#062.xml", WEIRD.as_bytes().to_vec()),
        ("PublishSettings.xml", PUBLISH.as_bytes().to_vec()),
        ("META-INF/metadata.xml", METADATA.as_bytes().to_vec()),
        ("basic.xfl", MARKER.as_bytes().to_vec()),
        ("bin/M 1 1700000001.dat", bitmap_dat()),
        ("bin/M 2 1700000002.dat", sound_dat()),
    ]
}
