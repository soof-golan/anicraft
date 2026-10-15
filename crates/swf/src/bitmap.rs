//! Bitmap characters: `DefineBitsLossless2` (zlib ARGB with premultiplied alpha) and JPEG passthrough.

use std::io::Write;

use anim_display::Image;
use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::bits::PutBytes;
use crate::error::SwfError;

/// `BitmapFormat` value for 32-bit ARGB.
const FORMAT_ARGB32: u8 = 5;

/// Compresses `data` with zlib.
///
/// # Errors
/// [`SwfError::Compression`] if the encoder fails (it only writes to memory, so this is not expected).
pub fn zlib(data: &[u8]) -> Result<Vec<u8>, SwfError> {
    let mut encoder = ZlibEncoder::new(Vec::with_capacity(data.len() / 2), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

/// The body of a `DefineBitsLossless2` tag for a premultiplied RGBA image: character id, format 5, size, then the
/// zlib-compressed ARGB pixels (the SWF byte order is alpha, red, green, blue).
///
/// # Errors
/// [`SwfError::BitmapTooLarge`] when a side exceeds 65535 pixels, [`SwfError::BitmapSize`] when the buffer does not
/// match the dimensions.
pub fn define_bits_lossless2_body(id: u16, image: &Image) -> Result<Vec<u8>, SwfError> {
    let (Ok(width), Ok(height)) = (u16::try_from(image.width), u16::try_from(image.height)) else {
        return Err(SwfError::BitmapTooLarge { width: image.width, height: image.height });
    };
    let expected = Image::byte_len(image.width, image.height).ok_or(SwfError::BitmapTooLarge { width: image.width, height: image.height })?;
    if image.premultiplied_rgba.len() != expected {
        return Err(SwfError::BitmapSize { width: image.width, height: image.height, expected, actual: image.premultiplied_rgba.len() });
    }
    let mut argb = Vec::with_capacity(expected);
    let (pixels, _) = image.premultiplied_rgba.as_chunks::<4>();
    for &[r, g, b, a] in pixels {
        argb.extend_from_slice(&[a, r, g, b]);
    }
    let mut body = Vec::new();
    body.put_u16(id);
    body.put_u8(FORMAT_ARGB32);
    body.put_u16(width);
    body.put_u16(height);
    body.extend_from_slice(&zlib(&argb)?);
    Ok(body)
}

/// The body of a `DefineBitsJPEG2` tag: character id followed by a complete JPEG stream (SOI to EOI).
#[must_use]
pub fn define_bits_jpeg2_body(id: u16, jpeg: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(jpeg.len().saturating_add(2));
    body.put_u16(id);
    body.extend_from_slice(jpeg);
    body
}

/// The body of a `DefineBitsJPEG3` tag: a JPEG for the colour plus one zlib-compressed alpha byte per pixel.
///
/// # Errors
/// [`SwfError::TooLarge`] if the JPEG is longer than 4 GiB.
pub fn define_bits_jpeg3_body(id: u16, jpeg: &[u8], alpha: &[u8]) -> Result<Vec<u8>, SwfError> {
    let jpeg_len = u32::try_from(jpeg.len()).map_err(|_| SwfError::TooLarge(jpeg.len()))?;
    let mut body = Vec::with_capacity(jpeg.len().saturating_add(alpha.len() / 2).saturating_add(8));
    body.put_u16(id);
    body.put_u32(jpeg_len);
    body.extend_from_slice(jpeg);
    body.extend_from_slice(&zlib(alpha)?);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::ZlibDecoder;

    use super::*;

    #[test]
    fn lossless2_stores_argb_in_zlib() {
        let image = Image::new(2, 1, vec![10, 20, 30, 255, 1, 2, 3, 4]).unwrap();
        let body = define_bits_lossless2_body(9, &image).unwrap();
        assert_eq!(&body[..7], &[9, 0, 5, 2, 0, 1, 0]);
        let mut raw = Vec::new();
        ZlibDecoder::new(&body[7..]).read_to_end(&mut raw).unwrap();
        assert_eq!(raw, vec![255, 10, 20, 30, 4, 1, 2, 3]);
    }

    #[test]
    fn size_checks() {
        let mut image = Image::new(1, 1, vec![0; 4]).unwrap();
        image.width = 70_000;
        assert!(matches!(define_bits_lossless2_body(1, &image), Err(SwfError::BitmapTooLarge { .. })));
        image.width = 2;
        assert!(matches!(define_bits_lossless2_body(1, &image), Err(SwfError::BitmapSize { .. })));
    }

    #[test]
    fn jpeg_bodies() {
        let body = define_bits_jpeg2_body(3, &[0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(body, vec![3, 0, 0xff, 0xd8, 0xff, 0xd9]);
        let body = define_bits_jpeg3_body(3, &[0xff, 0xd8, 0xff, 0xd9], &[255, 255]).unwrap();
        assert_eq!(&body[..8], &[3, 0, 4, 0, 0, 0, 0xff, 0xd8]);
    }
}
