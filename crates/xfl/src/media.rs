//! Bitmap and sound blobs.
//!
//! # Animate's lossless bitmap (`bin/M n timestamp.dat`)
//!
//! Reverse-engineered from the sample files; all integers are little endian.
//!
//! | offset | size | meaning |
//! |-------:|-----:|---------|
//! | 0 | 2 | magic `03 05` |
//! | 2 | 2 | row stride in bytes (`width * 4`) |
//! | 4 | 2 | width in pixels |
//! | 6 | 2 | height in pixels |
//! | 8 | 16 | four `u32`: left, right, top, bottom of the bitmap frame in twips (`right - left == width * 20`) |
//! | 24 | 1 | always 0 in the samples |
//! | 25 | 1 | 1 when the bitmap has transparency |
//! | 26 | .. | chunks until a zero length: `u16` length, then that many bytes |
//!
//! The chunk payloads (at most 2048 bytes each) concatenated form one zlib stream. It inflates to `height` rows of
//! `stride` bytes, each pixel stored as `A R G B`.
//!
//! Other `.dat` blobs are the original file (JPEG for photo bitmaps; MP4 for video, preceded by a small header) or,
//! for sounds, the PCM or MP3 data followed by a low-quality preview cache.

use std::io::{Cursor, Read};

use anim_display::Image;
use anim_scene::{MediaId, MediaStore, SoundItem};
use flate2::read::ZlibDecoder;

use crate::error::XflError;

/// Largest bitmap side accepted.
const MAX_SIDE: u32 = 32_768;
/// Largest decoded bitmap in bytes (`width * height * 4`).
const MAX_PIXEL_BYTES: usize = 1 << 30;
/// Size of the fixed header of the lossless format.
const LOSSLESS_HEADER: usize = 26;

/// What a stored bitmap blob is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitmapFormat {
    /// Animate's lossless `.dat` bitmap.
    Lossless,
    Jpeg,
    Png,
    Unknown,
}

/// Recognises the bitmap kind from the first bytes.
#[must_use]
pub const fn sniff_bitmap(bytes: &[u8]) -> BitmapFormat {
    match bytes {
        [0xFF, 0xD8, ..] => BitmapFormat::Jpeg,
        [0x89, b'P', b'N', b'G', ..] => BitmapFormat::Png,
        [0x03, 0x05, ..] => BitmapFormat::Lossless,
        _ => BitmapFormat::Unknown,
    }
}

/// Decodes a bitmap of the store into premultiplied RGBA.
///
/// # Errors
/// [`XflError::NoSuchMedia`] for an unknown id, [`XflError::Bitmap`] when the bytes are not a bitmap this reader
/// understands or are damaged.
pub fn decode_bitmap(store: &MediaStore, id: MediaId) -> Result<Image, XflError> {
    let bytes = store.get(id).ok_or(XflError::NoSuchMedia(id))?;
    decode_bitmap_bytes(bytes)
}

/// Decodes bitmap bytes (see [`decode_bitmap`]).
///
/// # Errors
/// See [`decode_bitmap`].
pub fn decode_bitmap_bytes(bytes: &[u8]) -> Result<Image, XflError> {
    match sniff_bitmap(bytes) {
        BitmapFormat::Lossless => decode_lossless(bytes),
        BitmapFormat::Jpeg | BitmapFormat::Png => decode_with_image_crate(bytes),
        BitmapFormat::Unknown => Err(XflError::Bitmap(format!("unknown bitmap format (starts with {:02x?})", bytes.get(..4).unwrap_or(bytes)))),
    }
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    let pair = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*pair.first()?, *pair.get(1)?]))
}

/// The header of a lossless bitmap blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LosslessHeader {
    pub stride: u32,
    pub width: u32,
    pub height: u32,
    pub has_transparency: bool,
}

/// Parses the fixed header of the lossless format.
///
/// # Errors
/// [`XflError::Bitmap`] when the header is truncated or its sizes are inconsistent or too large.
pub fn lossless_header(bytes: &[u8]) -> Result<LosslessHeader, XflError> {
    let bad = |what: &str| XflError::Bitmap(format!("lossless bitmap: {what}"));
    if bytes.len() < LOSSLESS_HEADER {
        return Err(bad("truncated header"));
    }
    let stride = u32::from(le16(bytes, 2).ok_or_else(|| bad("truncated header"))?);
    let width = u32::from(le16(bytes, 4).ok_or_else(|| bad("truncated header"))?);
    let height = u32::from(le16(bytes, 6).ok_or_else(|| bad("truncated header"))?);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(bad("bad dimensions"));
    }
    if Some(stride) != width.checked_mul(4) {
        return Err(bad("row stride is not width * 4"));
    }
    let has_transparency = bytes.get(25).is_some_and(|b| *b != 0);
    Ok(LosslessHeader { stride, width, height, has_transparency })
}

fn decode_lossless(bytes: &[u8]) -> Result<Image, XflError> {
    let header = lossless_header(bytes)?;
    let bad = |what: &str| XflError::Bitmap(format!("lossless bitmap: {what}"));
    let width = anim_core::num::u32_to_usize(header.width);
    let height = anim_core::num::u32_to_usize(header.height);
    let expected = width.checked_mul(height).and_then(|n| n.checked_mul(4)).filter(|n| *n <= MAX_PIXEL_BYTES).ok_or_else(|| bad("too large"))?;
    // Gather the chunk payloads into one zlib stream.
    let mut stream: Vec<u8> = Vec::new();
    let mut at = LOSSLESS_HEADER;
    loop {
        let length = usize::from(le16(bytes, at).ok_or_else(|| bad("truncated chunk header"))?);
        at = at.checked_add(2).ok_or_else(|| bad("offset overflow"))?;
        if length == 0 {
            break;
        }
        let end = at.checked_add(length).ok_or_else(|| bad("offset overflow"))?;
        let payload = bytes.get(at..end).ok_or_else(|| bad("truncated chunk"))?;
        stream.extend_from_slice(payload);
        if stream.len() > expected.saturating_mul(2).max(1 << 20) {
            return Err(bad("compressed data is larger than the bitmap"));
        }
        at = end;
    }
    let mut argb = Vec::with_capacity(expected);
    let wanted = u64::try_from(expected).unwrap_or(u64::MAX);
    ZlibDecoder::new(Cursor::new(&stream)).take(wanted.saturating_add(1)).read_to_end(&mut argb).map_err(|e| bad(&format!("inflate failed: {e}")))?;
    if argb.len() != expected {
        return Err(bad(&format!("inflated to {} bytes, expected {expected}", argb.len())));
    }
    // A pixel whose colour exceeds its alpha cannot be premultiplied, so the file holds straight alpha. When no pixel
    // gives it away the data is read as straight alpha too (Animate's own PNG export agrees on the samples).
    let mut rgba = Vec::with_capacity(expected);
    let (pixels, _) = argb.as_chunks::<4>();
    for &[alpha, red, green, blue] in pixels {
        let alpha = if header.has_transparency { alpha } else { 255 };
        rgba.extend_from_slice(&premultiply([red, green, blue], alpha));
    }
    Ok(Image::new(header.width, header.height, rgba)?)
}

/// Premultiplies straight-alpha `rgb` by `alpha`.
fn premultiply(rgb: [u8; 3], alpha: u8) -> [u8; 4] {
    let scale = |channel: u8| -> u8 {
        let v = (u16::from(channel).saturating_mul(u16::from(alpha)).saturating_add(127)) / 255;
        u8::try_from(v.min(255)).unwrap_or(255)
    };
    let [red, green, blue] = rgb;
    [scale(red), scale(green), scale(blue), alpha]
}

fn decode_with_image_crate(bytes: &[u8]) -> Result<Image, XflError> {
    let bad = |e: &dyn std::fmt::Display| XflError::Bitmap(format!("image decode failed: {e}"));
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| bad(&e))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(u64::try_from(MAX_PIXEL_BYTES).unwrap_or(u64::MAX));
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| bad(&e))?.into_rgba8();
    let (width, height) = decoded.dimensions();
    let mut out = Vec::with_capacity(decoded.as_raw().len());
    let (pixels, _) = decoded.as_raw().as_chunks::<4>();
    for &[red, green, blue, alpha] in pixels {
        out.extend_from_slice(&premultiply([red, green, blue], alpha));
    }
    Ok(Image::new(width, height, out)?)
}

/// The sound bytes proper of a sound item's `.dat`: Animate appends a low-quality preview cache after the
/// `dataLength` bytes of the real data.
#[must_use]
pub fn sound_payload<'a>(store: &'a MediaStore, item: &SoundItem) -> Option<&'a [u8]> {
    let bytes = store.get(item.media?)?;
    let length = item.extra.attr("dataLength").and_then(|v| v.trim().parse::<usize>().ok());
    Some(length.and_then(|n| bytes.get(..n)).unwrap_or(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    /// Builds a lossless blob of `width` x `height` from `argb` rows.
    fn lossless_blob(width: u16, height: u16, transparent: bool, argb: &[u8]) -> Vec<u8> {
        let mut out = vec![0x03, 0x05];
        out.extend_from_slice(&(width * 4).to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        for v in [0u32, u32::from(width) * 20, 0, u32::from(height) * 20] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[0, u8::from(transparent)]);
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(argb).unwrap();
        let packed = z.finish().unwrap();
        for chunk in packed.chunks(2048) {
            out.extend_from_slice(&u16::try_from(chunk.len()).unwrap().to_le_bytes());
            out.extend_from_slice(chunk);
        }
        out.extend_from_slice(&[0, 0]);
        out
    }

    #[test]
    fn decodes_a_small_lossless_bitmap() {
        let argb = [255, 10, 20, 30, 128, 100, 100, 100, 0, 0, 0, 0, 255, 255, 0, 0];
        let blob = lossless_blob(2, 2, true, &argb);
        assert_eq!(sniff_bitmap(&blob), BitmapFormat::Lossless);
        let image = decode_bitmap_bytes(&blob).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(&image.premultiplied_rgba[..4], &[10, 20, 30, 255]);
        // Straight (100,100,100) at alpha 128 premultiplies to 50.
        assert_eq!(&image.premultiplied_rgba[4..8], &[50, 50, 50, 128]);
        assert_eq!(&image.premultiplied_rgba[8..12], &[0, 0, 0, 0]);
    }

    #[test]
    fn opaque_bitmaps_ignore_stored_alpha() {
        let blob = lossless_blob(1, 1, false, &[7, 1, 2, 3]);
        assert_eq!(decode_bitmap_bytes(&blob).unwrap().premultiplied_rgba, vec![1, 2, 3, 255]);
    }

    #[test]
    fn large_bitmaps_span_many_chunks() {
        let argb: Vec<u8> = (0..(64u32 * 64 * 4)).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24 & 0xFF).unwrap()).collect();
        let blob = lossless_blob(64, 64, true, &argb);
        let image = decode_bitmap_bytes(&blob).unwrap();
        assert_eq!(image.premultiplied_rgba.len(), 64 * 64 * 4);
    }

    #[test]
    fn hostile_blobs_are_errors_not_panics() {
        let good = lossless_blob(4, 4, true, &[200; 64]);
        for cut in 0..good.len() {
            let _ = decode_bitmap_bytes(&good[..cut]);
        }
        let mut huge = good.clone();
        huge[4] = 0xFF;
        huge[5] = 0xFF;
        huge[2] = 0xFC;
        huge[3] = 0xFF;
        assert!(decode_bitmap_bytes(&huge).is_err());
        assert!(decode_bitmap_bytes(&[]).is_err());
        assert!(decode_bitmap_bytes(&[0x03, 0x05, 0, 0, 0, 0, 0, 0]).is_err());
        // A zlib bomb: tiny input, claims a small bitmap but inflates far beyond it.
        let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
        z.write_all(&vec![0u8; 10_000_000]).unwrap();
        let packed = z.finish().unwrap();
        let mut bomb = good[..26].to_vec();
        for chunk in packed.chunks(2048) {
            bomb.extend_from_slice(&u16::try_from(chunk.len()).unwrap().to_le_bytes());
            bomb.extend_from_slice(chunk);
        }
        bomb.extend_from_slice(&[0, 0]);
        assert!(decode_bitmap_bytes(&bomb).is_err());
    }

    #[test]
    fn png_and_garbage() {
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 128]).unwrap();
        }
        let image = decode_bitmap_bytes(&png_bytes).unwrap();
        assert_eq!(image.premultiplied_rgba, vec![128, 0, 0, 128]);
        assert!(decode_bitmap_bytes(b"not an image").is_err());
        assert!(decode_bitmap_bytes(&[0xFF, 0xD8, 0xFF]).is_err());
    }

    #[test]
    fn sound_payload_stops_before_the_cache() {
        let mut store = MediaStore::new();
        let id = store.insert(vec![1, 2, 3, 4, 5, 6]).unwrap();
        let mut item = SoundItem::new("s", "1");
        item.media = Some(id);
        assert_eq!(sound_payload(&store, &item), Some(&[1u8, 2, 3, 4, 5, 6][..]));
        item.extra.set_attr("dataLength", "4");
        assert_eq!(sound_payload(&store, &item), Some(&[1u8, 2, 3, 4][..]));
        item.extra.set_attr("dataLength", "99");
        assert_eq!(sound_payload(&store, &item), Some(&[1u8, 2, 3, 4, 5, 6][..]));
    }
}
