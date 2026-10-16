//! Byte and bit level writers for the SWF primitive types.
//!
//! SWF mixes little-endian byte fields with big-endian (MSB first) bit fields that are padded to a byte boundary
//! at the end of each record. [`PutBytes`] covers the former on `Vec<u8>`, [`BitWriter`] the latter.

use crate::error::SwfError;

/// Little-endian primitives appended to a byte buffer.
pub trait PutBytes {
    fn put_u8(&mut self, v: u8);
    fn put_u16(&mut self, v: u16);
    fn put_i16(&mut self, v: i16);
    fn put_u32(&mut self, v: u32);
    fn put_i32(&mut self, v: i32);
    fn put_f32(&mut self, v: f32);
    /// UTF-8 text followed by a NUL byte.
    ///
    /// # Errors
    /// [`SwfError::InteriorNul`] when `s` contains a NUL character.
    fn put_cstr(&mut self, s: &str) -> Result<(), SwfError>;
    /// The variable length `EncodedU32` (seven bits per byte, least significant group first).
    fn put_encoded_u32(&mut self, v: u32);
}

impl PutBytes for Vec<u8> {
    fn put_u8(&mut self, v: u8) {
        self.push(v);
    }

    fn put_u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn put_i16(&mut self, v: i16) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn put_u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn put_i32(&mut self, v: i32) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn put_f32(&mut self, v: f32) {
        self.extend_from_slice(&v.to_le_bytes());
    }

    fn put_cstr(&mut self, s: &str) -> Result<(), SwfError> {
        if s.contains('\0') {
            return Err(SwfError::InteriorNul);
        }
        self.extend_from_slice(s.as_bytes());
        self.push(0);
        Ok(())
    }

    fn put_encoded_u32(&mut self, mut v: u32) {
        loop {
            let low = u8::try_from(v & 0x7f).unwrap_or(0);
            v = v.wrapping_shr(7);
            if v == 0 {
                self.push(low);
                break;
            }
            self.push(low | 0x80);
        }
    }
}

/// An MSB-first bit writer; the unfinished byte is padded with zero bits by [`BitWriter::align`] and
/// [`BitWriter::into_bytes`].
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    /// The not yet written bits, in the low `filled` bits (always fewer than 8 between calls).
    acc: u64,
    filled: u32,
}

impl BitWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A writer that appends to `bytes`, which ends on a byte boundary.
    #[must_use]
    pub const fn appending_to(bytes: Vec<u8>) -> Self {
        Self { bytes, acc: 0, filled: 0 }
    }

    /// The low `bits` (at most 32) bits of `value`, most significant first.
    fn put(&mut self, value: u32, bits: u32) {
        let mask = 1_u64.wrapping_shl(bits).wrapping_sub(1);
        self.acc = self.acc.wrapping_shl(bits) | (u64::from(value) & mask);
        self.filled = self.filled.saturating_add(bits);
        while self.filled >= 8 {
            self.filled = self.filled.saturating_sub(8);
            self.bytes.push(u8::try_from(self.acc.wrapping_shr(self.filled) & 0xff).unwrap_or(0));
        }
        self.acc &= 1_u64.wrapping_shl(self.filled).wrapping_sub(1);
    }

    /// `count` copies of `bit`, for the extension of fields wider than 32 bits.
    fn put_run(&mut self, bit: bool, mut count: u32) {
        while count > 0 {
            let n = count.min(32);
            self.put(if bit { u32::MAX } else { 0 }, n);
            count = count.saturating_sub(n);
        }
    }

    /// One bit.
    pub fn bit(&mut self, value: bool) {
        self.put(u32::from(value), 1);
    }

    /// The low `bits` bits of `value`, most significant first (`UB[bits]`).
    pub fn ub(&mut self, value: u32, bits: u32) {
        self.put_run(false, bits.saturating_sub(32));
        self.put(value, bits.min(32));
    }

    /// `value` as a two's complement field of `bits` bits (`SB[bits]` and `FB[bits]`). Widths above 32 sign-extend.
    pub fn sb(&mut self, value: i32, bits: u32) {
        self.put_run(value < 0, bits.saturating_sub(32));
        self.put(u32::from_ne_bytes(value.to_ne_bytes()), bits.min(32));
    }

    /// Pads the current byte with zero bits.
    pub fn align(&mut self) {
        if self.filled != 0 {
            self.put(0, 8_u32.saturating_sub(self.filled));
        }
    }

    /// Pads to a byte boundary and returns the bytes.
    #[must_use]
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.align();
        self.bytes
    }
}

/// Bits needed for an unsigned value (0 needs 0 bits).
#[must_use]
pub const fn ub_bits(v: u32) -> u32 {
    u32::BITS.saturating_sub(v.leading_zeros())
}

/// Bits needed for a two's complement signed value (0 needs 1 bit).
#[must_use]
pub const fn sb_bits(v: i32) -> u32 {
    let magnitude = if v >= 0 { v } else { !v };
    ub_bits(u32::from_ne_bytes(magnitude.to_ne_bytes())).saturating_add(1)
}

/// The widest signed field needed by any of `values`.
#[must_use]
pub fn sb_bits_all(values: &[i32]) -> u32 {
    values.iter().map(|v| sb_bits(*v)).max().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_counts() {
        assert_eq!(ub_bits(0), 0);
        assert_eq!(ub_bits(1), 1);
        assert_eq!(ub_bits(255), 8);
        assert_eq!(ub_bits(256), 9);
        assert_eq!(sb_bits(0), 1);
        assert_eq!(sb_bits(1), 2);
        assert_eq!(sb_bits(-1), 1);
        assert_eq!(sb_bits(-2), 2);
        assert_eq!(sb_bits(3), 3);
        assert_eq!(sb_bits(-4), 3);
        assert_eq!(sb_bits(-5), 4);
        assert_eq!(sb_bits(i32::MAX), 32);
        assert_eq!(sb_bits(i32::MIN), 32);
    }

    #[test]
    fn bits_pack_msb_first_and_pad() {
        let mut w = BitWriter::new();
        w.ub(0b101, 3);
        w.sb(-2, 4);
        w.bit(true);
        assert_eq!(w.into_bytes(), vec![0b1011_1101]);
        let mut w = BitWriter::new();
        w.ub(0b11, 2);
        assert_eq!(w.into_bytes(), vec![0b1100_0000]);
    }

    /// The obvious one-bit-at-a-time writer the word-wise one replaced.
    fn naive(fields: &[(bool, i64, u32)]) -> Vec<u8> {
        let mut bits = Vec::new();
        for &(signed, value, width) in fields {
            for i in (0..width).rev() {
                bits.push(if i < 32 { (value >> i) & 1 == 1 } else { signed && value < 0 });
            }
        }
        bits.chunks(8)
            .map(|chunk| chunk.iter().enumerate().fold(0_u8, |byte, (i, &b)| byte | u8::from(b).wrapping_shl(7_u32.saturating_sub(u32::try_from(i).unwrap()))))
            .collect()
    }

    #[test]
    fn word_wise_writer_matches_the_bitwise_reference() {
        let values = [0_i64, 1, -1, 5, -5, 255, -256, 65_535, -65_536, i64::from(i32::MAX), i64::from(i32::MIN), 0x1234_5678];
        let mut fields = Vec::new();
        let mut w = BitWriter::new();
        let mut seed = 7_u32;
        for round in 0..400_u32 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = values[usize::try_from(seed >> 8).unwrap() % values.len()];
            let width = (seed >> 3) % 40;
            let signed = round % 2 == 0;
            if signed {
                w.sb(i32::try_from(value).unwrap(), width);
            } else {
                w.ub(u32::try_from(value & 0xffff_ffff).unwrap(), width);
            }
            fields.push((signed, if signed { value } else { value & 0xffff_ffff }, width));
        }
        assert_eq!(w.into_bytes(), naive(&fields));
    }

    #[test]
    fn signed_fields_round_trip_through_a_reader() {
        for (value, bits) in [(0, 1), (-1, 1), (5, 4), (-5, 4), (i32::MAX, 32), (i32::MIN, 32), (-65536, 17)] {
            let mut w = BitWriter::new();
            w.sb(value, bits);
            let bytes = w.into_bytes();
            let mut acc: i64 = 0;
            for i in 0..bits {
                let byte = bytes[usize::try_from(i / 8).unwrap()];
                let bit = (byte >> (7 - i % 8)) & 1;
                acc = (acc << 1) | i64::from(bit);
            }
            if acc >= 1_i64 << (bits - 1) {
                acc -= 1_i64 << bits;
            }
            assert_eq!(acc, i64::from(value), "{value} in {bits} bits");
        }
    }

    #[test]
    fn encoded_u32_matches_the_spec_examples() {
        let mut out = Vec::new();
        out.put_encoded_u32(0);
        out.put_encoded_u32(127);
        out.put_encoded_u32(128);
        out.put_encoded_u32(300);
        out.put_encoded_u32(u32::MAX);
        assert_eq!(out, vec![0x00, 0x7f, 0x80, 0x01, 0xac, 0x02, 0xff, 0xff, 0xff, 0xff, 0x0f]);
    }

    #[test]
    fn cstr_rejects_nul() {
        let mut out = Vec::new();
        out.put_cstr("héllo").unwrap();
        assert_eq!(out, b"h\xc3\xa9llo\0");
        assert!(matches!(out.put_cstr("a\0b"), Err(SwfError::InteriorNul)));
    }
}
