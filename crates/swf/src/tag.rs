//! Tag headers and the tag codes this crate writes.

use crate::bits::PutBytes;
use crate::error::SwfError;

/// Tag type codes.
pub mod code {
    pub const END: u16 = 0;
    pub const SHOW_FRAME: u16 = 1;
    pub const SET_BACKGROUND_COLOR: u16 = 9;
    pub const DO_ACTION: u16 = 12;
    pub const DEFINE_SOUND: u16 = 14;
    pub const START_SOUND: u16 = 15;
    pub const SOUND_STREAM_HEAD: u16 = 18;
    pub const SOUND_STREAM_BLOCK: u16 = 19;
    pub const DEFINE_BITS_JPEG2: u16 = 21;
    pub const DEFINE_SPRITE: u16 = 39;
    pub const FRAME_LABEL: u16 = 43;
    pub const SOUND_STREAM_HEAD2: u16 = 45;
    pub const DEFINE_BITS_LOSSLESS2: u16 = 36;
    pub const DEFINE_BITS_JPEG3: u16 = 35;
    pub const REMOVE_OBJECT2: u16 = 28;
    pub const PLACE_OBJECT2: u16 = 26;
    pub const FILE_ATTRIBUTES: u16 = 69;
    pub const PLACE_OBJECT3: u16 = 70;
    pub const SYMBOL_CLASS: u16 = 76;
    pub const METADATA: u16 = 77;
    pub const DEFINE_SHAPE4: u16 = 83;
    pub const DEFINE_SCENE_AND_FRAME_LABEL_DATA: u16 = 86;
    pub const DEFINE_MORPH_SHAPE2: u16 = 84;
    pub const DEFINE_BITS_JPEG4: u16 = 90;
    pub const EXPORT_ASSETS: u16 = 56;
    pub const DEFINE_SHAPE3: u16 = 32;
    pub const DEFINE_BUTTON2: u16 = 34;
    pub const DEFINE_BITS: u16 = 6;
    pub const DEFINE_BITS_LOSSLESS: u16 = 20;
    pub const DEFINE_SCALING_GRID: u16 = 78;
}

/// Tags that Flash Player requires to use the long header form whatever their size.
const fn must_be_long(tag_code: u16) -> bool {
    matches!(
        tag_code,
        code::DEFINE_BITS
            | code::DEFINE_BITS_JPEG2
            | code::DEFINE_BITS_JPEG3
            | code::DEFINE_BITS_JPEG4
            | code::DEFINE_BITS_LOSSLESS
            | code::DEFINE_BITS_LOSSLESS2
    )
}

/// Appends a complete tag (header and body) to `out`.
///
/// The short header (code and length in one `u16`) is used for bodies up to 62 bytes; longer bodies, and the bitmap
/// tags Flash Player insists on, get the long header with a 32-bit length.
///
/// # Errors
/// [`SwfError::TooLarge`] when the body is longer than `u32::MAX` bytes.
pub fn write_tag(out: &mut Vec<u8>, tag_code: u16, body: &[u8]) -> Result<(), SwfError> {
    let len = u32::try_from(body.len()).map_err(|_| SwfError::TooLarge(body.len()))?;
    let code_bits = (tag_code & 0x3ff).wrapping_shl(6);
    if len < 0x3f && !must_be_long(tag_code) {
        out.put_u16(code_bits | u16::try_from(len).unwrap_or(0));
    } else {
        out.put_u16(code_bits | 0x3f);
        out.put_u32(len);
    }
    out.extend_from_slice(body);
    Ok(())
}

/// A complete tag (header and body) as a fresh buffer.
///
/// # Errors
/// See [`write_tag`].
pub fn tag_bytes(tag_code: u16, body: &[u8]) -> Result<Vec<u8>, SwfError> {
    let mut out = Vec::with_capacity(body.len().saturating_add(6));
    write_tag(&mut out, tag_code, body)?;
    Ok(out)
}

/// One tag found by [`scan_tags`]: its code and body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawTag<'a> {
    pub code: u16,
    pub body: &'a [u8],
}

/// Splits a tag stream (the bytes after the header) into tags.
///
/// Stops at the `End` tag or at the end of the data. A truncated final tag is dropped. This is a minimal reader for
/// tests and tools; it does not interpret bodies.
#[must_use]
pub fn scan_tags(mut data: &[u8]) -> Vec<RawTag<'_>> {
    let mut tags = Vec::new();
    while let Some((head, rest)) = data.split_first_chunk::<2>() {
        let head = u16::from_le_bytes(*head);
        let tag_code = head.wrapping_shr(6);
        let short_len = usize::from(head & 0x3f);
        let (len, rest) = if short_len == 0x3f {
            let Some((len, rest)) = rest.split_first_chunk::<4>() else { break };
            (usize::try_from(u32::from_le_bytes(*len)).unwrap_or(usize::MAX), rest)
        } else {
            (short_len, rest)
        };
        let Some((body, rest)) = rest.split_at_checked(len) else { break };
        tags.push(RawTag { code: tag_code, body });
        data = rest;
        if tag_code == code::END {
            break;
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_long_headers() {
        let mut out = Vec::new();
        write_tag(&mut out, code::SHOW_FRAME, &[]).unwrap();
        assert_eq!(out, vec![0x40, 0x00]);

        let mut out = Vec::new();
        write_tag(&mut out, code::SET_BACKGROUND_COLOR, &[1, 2, 3]).unwrap();
        assert_eq!(out, vec![0x43, 0x02, 1, 2, 3]);

        // 62 bytes is the longest short body, 63 needs the long form
        let mut out = Vec::new();
        write_tag(&mut out, code::DO_ACTION, &[0; 62]).unwrap();
        assert_eq!(out.len(), 64);
        let mut out = Vec::new();
        write_tag(&mut out, code::DO_ACTION, &[0; 63]).unwrap();
        assert_eq!(&out[..6], &[0x3f, 0x03, 63, 0, 0, 0]);

        // bitmap tags always use the long form
        let mut out = Vec::new();
        write_tag(&mut out, code::DEFINE_BITS_LOSSLESS2, &[0; 4]).unwrap();
        assert_eq!(&out[..2], &[0x3f, 0x09]);
        assert_eq!(out.len(), 2 + 4 + 4);
    }

    #[test]
    fn scan_round_trips_and_survives_truncation() {
        let mut data = Vec::new();
        write_tag(&mut data, code::SET_BACKGROUND_COLOR, &[9, 8, 7]).unwrap();
        write_tag(&mut data, code::DEFINE_BITS_LOSSLESS2, &[5; 100]).unwrap();
        write_tag(&mut data, code::SHOW_FRAME, &[]).unwrap();
        write_tag(&mut data, code::END, &[]).unwrap();
        let tags = scan_tags(&data);
        assert_eq!(tags.iter().map(|t| t.code).collect::<Vec<_>>(), vec![9, 36, 1, 0]);
        assert_eq!(tags[1].body.len(), 100);
        for cut in 0..data.len() {
            let _ = scan_tags(&data[..cut]);
        }
    }
}
