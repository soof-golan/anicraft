//! Sounds: `DefineSound` (MP3 passthrough or 16-bit PCM), `StartSound`, and MP3 streaming blocks for the timeline.
//!
//! MP3 data is not decoded: the frame headers are parsed only to learn the sample rate, the channel count and how
//! many samples each frame holds, so that the stream can be cut into per-frame `SoundStreamBlock`s.

use crate::bits::PutBytes;
use crate::error::SwfError;

/// Sound format code for MP3.
const FORMAT_MP3: u8 = 2;
/// Sound format code for uncompressed little-endian PCM.
const FORMAT_PCM_LE: u8 = 3;

/// One MP3 frame found in a byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp3Frame {
    pub offset: usize,
    pub len: usize,
    /// Samples per channel the frame decodes to (1152 for MPEG-1, 576 for MPEG-2/2.5 layer III).
    pub samples: u32,
}

/// What [`parse_mp3`] learned about a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp3Info {
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: Vec<Mp3Frame>,
}

impl Mp3Info {
    /// Samples per channel in the whole stream.
    #[must_use]
    pub fn total_samples(&self) -> u32 {
        self.frames.iter().fold(0_u32, |acc, f| acc.saturating_add(f.samples))
    }

    /// Offset of the first frame and end of the last, i.e. the byte range that holds audio.
    #[must_use]
    pub fn audio_range(&self) -> (usize, usize) {
        let start = self.frames.first().map_or(0, |f| f.offset);
        let end = self.frames.last().map_or(0, |f| f.offset.saturating_add(f.len));
        (start, end)
    }
}

const MPEG1_L3_BITRATES: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const MPEG2_L3_BITRATES: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// Parses the header at the start of `data`: (frame length, sample rate, channels, samples per frame).
fn parse_frame_header(data: &[u8]) -> Option<(usize, u32, u8, u32)> {
    let &[b0, b1, b2, b3, ..] = data else { return None };
    if b0 != 0xff || (b1 & 0xe0) != 0xe0 {
        return None;
    }
    let version = (b1 >> 3) & 0x3; // 0: 2.5, 1: reserved, 2: MPEG-2, 3: MPEG-1
    let layer = (b1 >> 1) & 0x3; // 1: layer III
    if version == 1 || layer != 1 {
        return None;
    }
    let bitrate_index = usize::from(b2 >> 4);
    let rate_index = usize::from((b2 >> 2) & 0x3);
    if bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    let padding = u32::from((b2 >> 1) & 1);
    let mpeg1 = version == 3;
    let table = if mpeg1 { &MPEG1_L3_BITRATES } else { &MPEG2_L3_BITRATES };
    let bitrate = table.get(bitrate_index)?.checked_mul(1000)?;
    let base_rate = *[44_100_u32, 48_000, 32_000].get(rate_index)?;
    let sample_rate = match version {
        3 => base_rate,
        2 => base_rate / 2,
        _ => base_rate / 4,
    };
    let (coefficient, samples) = if mpeg1 { (144_u32, 1152_u32) } else { (72, 576) };
    let len = coefficient.checked_mul(bitrate)?.checked_div(sample_rate)?.checked_add(padding)?;
    let channels = if (b3 >> 6) == 3 { 1 } else { 2 };
    Some((usize::try_from(len).ok()?, sample_rate, channels, samples))
}

/// Finds the layer III frames in `data`, skipping an `ID3v2` tag at the start and any junk between frames.
///
/// # Errors
/// [`SwfError::InvalidMp3`] when no frame is found, or when frames disagree about the sample rate.
pub fn parse_mp3(data: &[u8]) -> Result<Mp3Info, SwfError> {
    let mut pos = skip_id3v2(data);
    let mut frames = Vec::new();
    let mut format: Option<(u32, u8)> = None;
    while let Some(rest) = data.get(pos..) {
        let Some((len, rate, channels, samples)) = parse_frame_header(rest) else {
            pos = pos.saturating_add(1);
            if pos >= data.len() {
                break;
            }
            continue;
        };
        if len < 4 || pos.saturating_add(len) > data.len() {
            // A truncated last frame is dropped; a bogus length mid-stream means this was not a real header.
            pos = pos.saturating_add(1);
            continue;
        }
        // A real header is followed by another header (or by the end of the data).
        let next = pos.saturating_add(len);
        let followed = next >= data.len() || data.get(next..).and_then(parse_frame_header).is_some() || is_trailer(data.get(next..).unwrap_or(&[]));
        if !followed {
            pos = pos.saturating_add(1);
            continue;
        }
        match format {
            None => format = Some((rate, channels)),
            Some((r, _)) if r != rate => return Err(SwfError::InvalidMp3("the sample rate changes inside the stream")),
            Some(_) => {}
        }
        frames.push(Mp3Frame { offset: pos, len, samples });
        pos = next;
    }
    let (sample_rate, channels) = format.ok_or(SwfError::InvalidMp3("no layer III frame found"))?;
    Ok(Mp3Info { sample_rate, channels, frames })
}

/// An `ID3v1` tag or `ID3v2` footer that legitimately follows the last frame.
fn is_trailer(rest: &[u8]) -> bool {
    rest.starts_with(b"TAG") || rest.starts_with(b"3DI") || rest.iter().all(|b| *b == 0)
}

/// Length of a leading `ID3v2` tag (0 if there is none).
fn skip_id3v2(data: &[u8]) -> usize {
    let Some(&[b'I', b'D', b'3', _, _, flags, s0, s1, s2, s3]) = data.first_chunk::<10>() else { return 0 };
    let size = [s0, s1, s2, s3].iter().fold(0_usize, |acc, b| acc.saturating_mul(128).saturating_add(usize::from(b & 0x7f)));
    let footer = if flags & 0x10 != 0 { 10 } else { 0 };
    10_usize.saturating_add(size).saturating_add(footer).min(data.len())
}

/// The SWF `SoundRate` code (0: 5.5 kHz, 1: 11, 2: 22, 3: 44) closest to `rate` from above.
const fn rate_code(rate: u32) -> u8 {
    if rate >= 32_000 {
        3
    } else if rate >= 16_000 {
        2
    } else if rate >= 8_000 {
        1
    } else {
        0
    }
}

/// The body of a `DefineSound` tag holding MP3 data.
///
/// # Errors
/// [`SwfError::InvalidMp3`] if `data` has no usable frames.
pub fn define_sound_mp3_body(id: u16, data: &[u8]) -> Result<Vec<u8>, SwfError> {
    let info = parse_mp3(data)?;
    let (start, end) = info.audio_range();
    let audio = data.get(start..end).ok_or(SwfError::InvalidMp3("frame range outside the data"))?;
    let mut body = Vec::with_capacity(audio.len().saturating_add(10));
    body.put_u16(id);
    body.put_u8(FORMAT_MP3.wrapping_shl(4) | rate_code(info.sample_rate).wrapping_shl(2) | 0b10 | u8::from(info.channels == 2));
    body.put_u32(info.total_samples());
    body.put_i16(0); // seek samples
    body.extend_from_slice(audio);
    Ok(body)
}

/// The body of a `DefineSound` tag holding uncompressed 16-bit PCM.
///
/// `samples` are interleaved when `channels` is 2. SWF only plays 5512, 11025, 22050 and 44100 Hz PCM.
///
/// # Errors
/// [`SwfError::UnsupportedSound`] for another rate or channel count, [`SwfError::TooLarge`] for absurd lengths.
pub fn define_sound_pcm_body(id: u16, samples: &[i16], channels: u8, sample_rate: u32) -> Result<Vec<u8>, SwfError> {
    let code = match sample_rate {
        5_512 | 5_500 => 0_u8,
        11_025 => 1,
        22_050 => 2,
        44_100 => 3,
        _ => return Err(SwfError::UnsupportedSound("PCM must be 5512, 11025, 22050 or 44100 Hz")),
    };
    if channels == 0 || channels > 2 {
        return Err(SwfError::UnsupportedSound("PCM must be mono or stereo"));
    }
    let per_channel = samples.len().checked_div(usize::from(channels)).unwrap_or(0);
    let count = u32::try_from(per_channel).map_err(|_| SwfError::TooLarge(per_channel))?;
    let mut body = Vec::with_capacity(samples.len().saturating_mul(2).saturating_add(8));
    body.put_u16(id);
    body.put_u8(FORMAT_PCM_LE.wrapping_shl(4) | code.wrapping_shl(2) | 0b10 | u8::from(channels == 2));
    body.put_u32(count);
    for s in samples {
        body.put_i16(*s);
    }
    Ok(body)
}

/// `SOUNDINFO` of a `StartSound` tag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SoundInfo {
    /// Stop the sound instead of starting it.
    pub stop: bool,
    /// Do not start if this sound is already playing.
    pub no_multiple: bool,
    /// First sample to play.
    pub in_point: Option<u32>,
    /// Sample to stop at.
    pub out_point: Option<u32>,
    pub loops: Option<u16>,
    /// Volume envelope: `(sample position at 44.1 kHz, left, right)` with levels 0 ..= 32768.
    pub envelope: Vec<(u32, u16, u16)>,
}

/// The body of a `StartSound` tag. Envelopes longer than 255 points are truncated.
#[must_use]
pub fn start_sound_body(sound: u16, info: &SoundInfo) -> Vec<u8> {
    let mut body = Vec::new();
    body.put_u16(sound);
    let envelope: Vec<_> = info.envelope.iter().take(255).collect();
    body.put_u8(
        u8::from(info.stop).wrapping_shl(5)
            | u8::from(info.no_multiple).wrapping_shl(4)
            | u8::from(!envelope.is_empty()).wrapping_shl(3)
            | u8::from(info.loops.is_some()).wrapping_shl(2)
            | u8::from(info.out_point.is_some()).wrapping_shl(1)
            | u8::from(info.in_point.is_some()),
    );
    if let Some(v) = info.in_point {
        body.put_u32(v);
    }
    if let Some(v) = info.out_point {
        body.put_u32(v);
    }
    if let Some(v) = info.loops {
        body.put_u16(v);
    }
    if !envelope.is_empty() {
        body.put_u8(u8::try_from(envelope.len()).unwrap_or(255));
        for (pos, left, right) in envelope {
            body.put_u32(*pos);
            body.put_u16(*left);
            body.put_u16(*right);
        }
    }
    body
}

/// A streaming MP3 sound track attached to a timeline: one `SoundStreamHead2` and a block per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp3Stream {
    /// Sample rate of the MP3 data.
    pub sample_rate: u32,
    pub stereo: bool,
    /// Average samples per timeline frame (the head's advertised value).
    pub samples_per_frame: u16,
    /// `SoundStreamBlock` bodies, one per timeline frame from the frame that carries the head onwards. Frames past
    /// the end of the audio have no block.
    pub blocks: Vec<Vec<u8>>,
}

impl Mp3Stream {
    /// The `SoundStreamHead2` tag body.
    #[must_use]
    pub fn head_body(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(6);
        // playback format mirrors the stream format; both are 16-bit
        let rate = rate_code(self.sample_rate);
        let stereo = u8::from(self.stereo);
        body.put_u8(rate.wrapping_shl(2) | 0b10 | stereo);
        body.put_u8(FORMAT_MP3.wrapping_shl(4) | rate.wrapping_shl(2) | 0b10 | stereo);
        body.put_u16(self.samples_per_frame);
        body.put_i16(0); // latency seek
        body
    }
}

/// Cuts an MP3 file into per-frame blocks for a stream at `frame_rate` frames per second, at most `max_frames`
/// blocks long.
///
/// # Errors
/// [`SwfError::InvalidMp3`] for an unusable stream or a frame rate so low that a block would exceed 65535 samples;
/// [`SwfError::FrameRate`] for a non-positive frame rate.
pub fn split_mp3_stream(data: &[u8], frame_rate: f64, max_frames: usize) -> Result<Mp3Stream, SwfError> {
    if !(frame_rate.is_finite() && frame_rate > 0.0) {
        return Err(SwfError::FrameRate(frame_rate));
    }
    let info = parse_mp3(data)?;
    let samples_per_frame = f64::from(info.sample_rate) / frame_rate;
    let advertised =
        u16::try_from(anim_core::num::round_f64_to_u32(samples_per_frame)).map_err(|_| SwfError::InvalidMp3("frame rate too low for streaming"))?;
    let mut blocks = Vec::new();
    let mut next = 0_usize;
    let mut consumed_samples = 0_u32;
    for frame_index in 0..max_frames {
        if next >= info.frames.len() {
            break;
        }
        let target = anim_core::num::round_f64_to_u32(samples_per_frame * anim_core::num::usize_to_f64(frame_index.saturating_add(1)));
        let first = next;
        let mut block_samples = 0_u32;
        while let Some(frame) = info.frames.get(next) {
            if consumed_samples >= target {
                break;
            }
            consumed_samples = consumed_samples.saturating_add(frame.samples);
            block_samples = block_samples.saturating_add(frame.samples);
            next = next.saturating_add(1);
        }
        let mut body = Vec::new();
        body.put_u16(u16::try_from(block_samples).map_err(|_| SwfError::InvalidMp3("frame rate too low for streaming"))?);
        body.put_i16(0);
        for frame in info.frames.get(first..next).unwrap_or(&[]) {
            if let Some(bytes) = data.get(frame.offset..frame.offset.saturating_add(frame.len)) {
                body.extend_from_slice(bytes);
            }
        }
        blocks.push(body);
    }
    Ok(Mp3Stream { sample_rate: info.sample_rate, stereo: info.channels == 2, samples_per_frame: advertised, blocks })
}

#[cfg(test)]
pub(crate) mod test_data {
    /// A silent MPEG-1 layer III 128 kbps 44.1 kHz stereo frame (417 bytes).
    pub fn frame() -> Vec<u8> {
        let mut f = vec![0xff, 0xfb, 0x90, 0x00];
        f.resize(417, 0);
        f
    }

    /// `n` frames, optionally preceded by an `ID3v2` tag.
    pub fn stream(n: usize, id3: bool) -> Vec<u8> {
        let mut data = Vec::new();
        if id3 {
            data.extend_from_slice(b"ID3\x03\x00\x00\x00\x00\x00\x05hello");
        }
        for _ in 0..n {
            data.extend_from_slice(&frame());
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::test_data::{frame, stream};
    use super::*;

    #[test]
    fn parses_frames_and_skips_id3() {
        let info = parse_mp3(&stream(5, true)).unwrap();
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.frames.len(), 5);
        assert_eq!(info.frames[0].offset, 15);
        assert_eq!(info.frames[1].offset, 15 + 417);
        assert_eq!(info.total_samples(), 5 * 1152);
    }

    #[test]
    fn junk_and_truncation_are_tolerated() {
        let mut data = vec![0, 1, 2, 3];
        data.extend_from_slice(&stream(3, false));
        data.extend_from_slice(&frame()[..100]); // truncated last frame
        let info = parse_mp3(&data).unwrap();
        assert_eq!(info.frames.len(), 3);
        assert!(matches!(parse_mp3(&[1, 2, 3, 4, 5]), Err(SwfError::InvalidMp3(_))));
        assert!(parse_mp3(&[]).is_err());
        for cut in 0..data.len() {
            let _ = parse_mp3(&data[..cut]);
        }
    }

    #[test]
    fn define_sound_header() {
        let body = define_sound_mp3_body(5, &stream(4, false)).unwrap();
        assert_eq!(&body[..2], &[5, 0]);
        // MP3 (0010), 44k (11), 16 bit (1), stereo (1)
        assert_eq!(body[2], 0b0010_1111);
        assert_eq!(u32::from_le_bytes(body[3..7].try_into().unwrap()), 4 * 1152);
        assert_eq!(&body[7..9], &[0, 0]);
        assert_eq!(body.len(), 9 + 4 * 417);
    }

    #[test]
    fn pcm_sounds_validate_the_rate() {
        let body = define_sound_pcm_body(1, &[1, -1, 2, -2], 2, 44_100).unwrap();
        assert_eq!(body[2], 0b0011_1111);
        assert_eq!(u32::from_le_bytes(body[3..7].try_into().unwrap()), 2);
        assert_eq!(body.len(), 7 + 8);
        assert!(define_sound_pcm_body(1, &[0], 1, 48_000).is_err());
        assert!(define_sound_pcm_body(1, &[0], 3, 44_100).is_err());
    }

    #[test]
    fn start_sound_flags() {
        let info = SoundInfo { loops: Some(3), in_point: Some(10), envelope: vec![(0, 100, 200)], ..SoundInfo::default() };
        let body = start_sound_body(2, &info);
        assert_eq!(body[2], 0b0000_1101);
        assert_eq!(&body[3..7], &10_u32.to_le_bytes());
        assert_eq!(&body[7..9], &3_u16.to_le_bytes());
        assert_eq!(body[9], 1);
        assert_eq!(body.len(), 10 + 8);
    }

    #[test]
    fn stream_blocks_cover_all_audio_at_the_frame_rate() {
        // 44100 Hz at 24 fps = 1837.5 samples per frame: one or two 1152-sample MP3 frames per block
        let data = stream(40, false);
        let s = split_mp3_stream(&data, 24.0, 1000).unwrap();
        assert_eq!(s.samples_per_frame, 1838);
        assert!(s.stereo);
        let total: u32 = s.blocks.iter().map(|b| u32::from(u16::from_le_bytes([b[0], b[1]]))).sum();
        assert_eq!(total, 40 * 1152);
        let bytes: usize = s.blocks.iter().map(|b| b.len() - 4).sum();
        assert_eq!(bytes, 40 * 417);
        // 40 * 1152 / 1837.5 = 25.07 frames of audio
        assert!(s.blocks.len() >= 25 && s.blocks.len() <= 27, "{}", s.blocks.len());
        assert_eq!(s.head_body().len(), 6);
        // limited by max_frames
        assert_eq!(split_mp3_stream(&data, 24.0, 3).unwrap().blocks.len(), 3);
        assert!(split_mp3_stream(&data, 0.0, 3).is_err());
        assert!(split_mp3_stream(&data, 0.5, 3).is_err());
    }
}
