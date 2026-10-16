//! The movie builder: header, dictionary (characters) and timeline.
//!
//! [`Movie`] appends tags in order, so definitions can be interleaved with frames (a frame by frame export defines
//! new shapes just before the frame that first uses them). Identical shapes and sprites are written once.

use anim_core::{FrameRate, Rgba8, num};
use anim_display::Image;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::bitmap::{define_bits_jpeg2_body, define_bits_jpeg3_body, define_bits_lossless2_body, zlib};
use crate::bits::PutBytes;
use crate::button::ButtonDef;
use crate::error::SwfError;
use crate::morph::MorphShapeDef;
use crate::place::{Placement, remove_object2_body};
use crate::shape::ShapeDef;
use crate::sound::{SoundInfo, define_sound_mp3_body, define_sound_pcm_body, resample_pcm, start_sound_body, swf_pcm_rate};
use crate::tag::{code, write_tag, write_tag_after};
use crate::types::{CharacterId, Rect, put_rgb, to_twips};

/// SWF version written by default (Flash Player 19).
pub const DEFAULT_VERSION: u8 = 19;
/// Lowest version this writer targets: `DefineShape4`, filters, blend modes and the non-zero winding flag all exist.
pub const MIN_VERSION: u8 = 10;

/// Bytecode for `stop()` in the first script virtual machine (with the terminating zero).
pub const ACTION_STOP: [u8; 2] = [0x07, 0x00];
/// Bytecode for `play()` in the first script virtual machine (with the terminating zero).
pub const ACTION_PLAY: [u8; 2] = [0x06, 0x00];

/// A small XMP packet for [`MovieSettings::metadata`]: the document title, the media type and the creator tool.
#[must_use]
pub fn xmp_packet(title: &str) -> String {
    let title = title.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
         <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\
         <dc:format>application/x-shockwave-flash</dc:format><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{title}</rdf:li></rdf:Alt></dc:title>\
         <xmp:CreatorTool>AniCraft</xmp:CreatorTool></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
    )
}

/// Whether the file body is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Compression {
    /// `FWS`
    None,
    /// `CWS`: zlib
    #[default]
    Zlib,
}

/// Movie-level settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovieSettings {
    /// Stage width in pixels.
    pub width: f64,
    /// Stage height in pixels.
    pub height: f64,
    /// Frames per second (0 < rate < 256; SWF stores 8.8 fixed point).
    pub frame_rate: f64,
    /// Background colour (alpha is ignored; SWF backgrounds are opaque).
    pub background: Rgba8,
    pub version: u8,
    pub compression: Compression,
    /// An XMP packet to store as the movie's metadata.
    pub metadata: Option<String>,
    /// Allow network access instead of local file access only.
    pub use_network: bool,
}

impl Default for MovieSettings {
    fn default() -> Self {
        Self {
            width: 550.0,
            height: 400.0,
            frame_rate: 24.0,
            background: Rgba8::WHITE,
            version: DEFAULT_VERSION,
            compression: Compression::Zlib,
            metadata: None,
            use_network: false,
        }
    }
}

impl MovieSettings {
    /// Settings for a stage of `width` x `height` pixels at a rational frame rate.
    #[must_use]
    pub fn with_frame_rate(width: f64, height: f64, rate: FrameRate) -> Self {
        let fps = if rate.den == 0 { 24.0 } else { num::i64_to_f64(rate.num) / num::i64_to_f64(rate.den) };
        Self { width, height, frame_rate: fps, ..Self::default() }
    }
}

/// One item of a frame, in the order it appears in the tag stream.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameItem {
    Place(Placement),
    /// `RemoveObject2` at a depth.
    Remove(u16),
    StartSound {
        sound: CharacterId,
        info: SoundInfo,
    },
    /// A `SoundStreamHead2` body (see [`crate::sound::Mp3Stream::head_body`]).
    StreamHead(Vec<u8>),
    /// A `SoundStreamBlock` body.
    StreamBlock(Vec<u8>),
    /// Script bytecode for the first virtual machine; must end with a zero byte (see [`ACTION_STOP`]).
    DoAction(Vec<u8>),
    /// Any other tag.
    Raw {
        code: u16,
        body: Vec<u8>,
    },
}

/// One frame of a timeline: an optional label and the items that run before `ShowFrame`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub label: Option<String>,
    pub items: Vec<FrameItem>,
}

impl Frame {
    #[must_use]
    pub const fn new(items: Vec<FrameItem>) -> Self {
        Self { label: None, items }
    }
}

/// Appends the tags of `frame` followed by `ShowFrame`.
fn encode_frame(out: &mut Vec<u8>, frame: &Frame) -> Result<(), SwfError> {
    if let Some(label) = &frame.label {
        let mut body = Vec::new();
        body.put_cstr(label)?;
        write_tag(out, code::FRAME_LABEL, &body)?;
    }
    for item in &frame.items {
        match item {
            FrameItem::Place(placement) => write_tag(out, code::PLACE_OBJECT3, &placement.encode()?)?,
            FrameItem::Remove(depth) => write_tag(out, code::REMOVE_OBJECT2, &remove_object2_body(*depth))?,
            FrameItem::StartSound { sound, info } => write_tag(out, code::START_SOUND, &start_sound_body(sound.0, info))?,
            FrameItem::StreamHead(body) => write_tag(out, code::SOUND_STREAM_HEAD2, body)?,
            FrameItem::StreamBlock(body) => write_tag(out, code::SOUND_STREAM_BLOCK, body)?,
            FrameItem::DoAction(body) => write_tag(out, code::DO_ACTION, body)?,
            FrameItem::Raw { code, body } => write_tag(out, *code, body)?,
        }
    }
    write_tag(out, code::SHOW_FRAME, &[])
}

/// Which of the movie's tables of already defined characters a definition belongs to.
#[derive(Clone, Copy)]
enum Dedup {
    Shape,
    Morph,
    Sprite,
}

/// A SWF under construction.
#[derive(Debug)]
pub struct Movie {
    settings: MovieSettings,
    /// Definition and frame tags so far.
    body: Vec<u8>,
    next_id: u32,
    frames: u32,
    shapes: FxHashMap<Vec<u8>, CharacterId>,
    morphs: FxHashMap<Vec<u8>, CharacterId>,
    sprites: FxHashMap<Vec<u8>, CharacterId>,
    scenes: Vec<(u32, String)>,
    labels: Vec<(u32, String)>,
    exports: Vec<(CharacterId, String)>,
}

impl Movie {
    /// Starts a movie.
    ///
    /// # Errors
    /// [`SwfError::StageSize`] and [`SwfError::FrameRate`] for settings SWF cannot hold.
    pub fn new(mut settings: MovieSettings) -> Result<Self, SwfError> {
        if !(settings.width.is_finite() && settings.height.is_finite() && settings.width > 0.0 && settings.height > 0.0) {
            return Err(SwfError::StageSize { width: settings.width, height: settings.height });
        }
        if !(settings.frame_rate.is_finite() && settings.frame_rate > 0.0 && settings.frame_rate < 256.0) {
            return Err(SwfError::FrameRate(settings.frame_rate));
        }
        settings.version = settings.version.max(MIN_VERSION);
        Ok(Self {
            settings,
            body: Vec::new(),
            next_id: 1,
            frames: 0,
            shapes: FxHashMap::default(),
            morphs: FxHashMap::default(),
            sprites: FxHashMap::default(),
            scenes: Vec::new(),
            labels: Vec::new(),
            exports: Vec::new(),
        })
    }

    #[must_use]
    pub const fn settings(&self) -> &MovieSettings {
        &self.settings
    }

    /// Frames added to the main timeline so far.
    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frames
    }

    /// Characters defined so far.
    #[must_use]
    pub const fn character_count(&self) -> u32 {
        self.next_id.saturating_sub(1)
    }

    /// Bytes of tags written so far (before compression).
    #[must_use]
    pub const fn size_so_far(&self) -> usize {
        self.body.len()
    }

    /// Reserves the next character id.
    ///
    /// # Errors
    /// [`SwfError::TooManyCharacters`] after 65535 characters.
    pub fn alloc_id(&mut self) -> Result<CharacterId, SwfError> {
        let id = u16::try_from(self.next_id).map_err(|_| SwfError::TooManyCharacters)?;
        self.next_id = self.next_id.saturating_add(1);
        Ok(CharacterId(id))
    }

    /// Defines a shape (`DefineShape4`); a shape identical to an earlier one returns that one's id.
    ///
    /// # Errors
    /// See [`ShapeDef::encode`] and [`SwfError::TooManyCharacters`].
    pub fn define_shape(&mut self, shape: &ShapeDef) -> Result<CharacterId, SwfError> {
        let encoded = shape.encode()?;
        self.define_once(Dedup::Shape, code::DEFINE_SHAPE4, encoded)
    }

    /// Writes the definition tag `tag_code` for `encoded` (the body after the character id) unless the same bytes were
    /// defined before in the same table; returns the character either way.
    fn define_once(&mut self, table: Dedup, tag_code: u16, encoded: Vec<u8>) -> Result<CharacterId, SwfError> {
        let known = match table {
            Dedup::Shape => &mut self.shapes,
            Dedup::Morph => &mut self.morphs,
            Dedup::Sprite => &mut self.sprites,
        };
        if let Some(id) = known.get(&encoded) {
            return Ok(*id);
        }
        let id = u16::try_from(self.next_id).map(CharacterId).map_err(|_| SwfError::TooManyCharacters)?;
        self.next_id = self.next_id.saturating_add(1);
        write_tag_after(&mut self.body, tag_code, &id.0.to_le_bytes(), &encoded)?;
        known.insert(encoded, id);
        Ok(id)
    }

    /// Defines a shape tween (`DefineMorphShape2`); place it with a `ratio` from 0 to 65535 to show the blend.
    ///
    /// # Errors
    /// See [`MorphShapeDef::encode`] and [`SwfError::TooManyCharacters`].
    pub fn define_morph_shape(&mut self, morph: &MorphShapeDef) -> Result<CharacterId, SwfError> {
        let encoded = morph.encode()?;
        self.define_once(Dedup::Morph, code::DEFINE_MORPH_SHAPE2, encoded)
    }

    /// Defines a button (`DefineButton2`).
    ///
    /// # Errors
    /// See [`ButtonDef::encode`] and [`SwfError::TooManyCharacters`].
    pub fn define_button(&mut self, button: &ButtonDef) -> Result<CharacterId, SwfError> {
        let id = self.alloc_id()?;
        write_tag_after(&mut self.body, code::DEFINE_BUTTON2, &id.0.to_le_bytes(), &button.encode()?)?;
        Ok(id)
    }

    /// Defines a lossless bitmap (`DefineBitsLossless2`).
    ///
    /// # Errors
    /// See [`define_bits_lossless2_body`].
    pub fn define_bitmap(&mut self, image: &Image) -> Result<CharacterId, SwfError> {
        let id = self.alloc_id()?;
        write_tag(&mut self.body, code::DEFINE_BITS_LOSSLESS2, &define_bits_lossless2_body(id.0, image)?)?;
        Ok(id)
    }

    /// Defines a JPEG bitmap; with `alpha` (one byte per pixel) it becomes `DefineBitsJPEG3`.
    ///
    /// # Errors
    /// Compression or size failures.
    pub fn define_jpeg(&mut self, jpeg: &[u8], alpha: Option<&[u8]>) -> Result<CharacterId, SwfError> {
        let id = self.alloc_id()?;
        match alpha {
            Some(alpha) => write_tag(&mut self.body, code::DEFINE_BITS_JPEG3, &define_bits_jpeg3_body(id.0, jpeg, alpha)?)?,
            None => write_tag(&mut self.body, code::DEFINE_BITS_JPEG2, &define_bits_jpeg2_body(id.0, jpeg))?,
        }
        Ok(id)
    }

    /// Defines an event sound from MP3 data (passed through undecoded).
    ///
    /// # Errors
    /// [`SwfError::InvalidMp3`] when no layer III frames are found.
    pub fn define_sound_mp3(&mut self, mp3: &[u8]) -> Result<CharacterId, SwfError> {
        let id = self.alloc_id()?;
        write_tag(&mut self.body, code::DEFINE_SOUND, &define_sound_mp3_body(id.0, mp3)?)?;
        Ok(id)
    }

    /// Defines an event sound from 16-bit PCM (interleaved when stereo; 5512, 11025, 22050 or 44100 Hz).
    ///
    /// # Errors
    /// [`SwfError::UnsupportedSound`] for other rates.
    pub fn define_sound_pcm(&mut self, samples: &[i16], channels: u8, sample_rate: u32) -> Result<CharacterId, SwfError> {
        let id = self.alloc_id()?;
        write_tag(&mut self.body, code::DEFINE_SOUND, &define_sound_pcm_body(id.0, samples, channels, sample_rate)?)?;
        Ok(id)
    }

    /// Like [`Movie::define_sound_pcm`] for any sample rate: sounds at other rates are resampled to the nearest rate
    /// SWF plays (see [`swf_pcm_rate`]).
    ///
    /// # Errors
    /// [`SwfError::UnsupportedSound`] for zero rates or channel counts, or a mono/stereo mismatch.
    pub fn define_sound_pcm_any_rate(&mut self, samples: &[i16], channels: u8, sample_rate: u32) -> Result<CharacterId, SwfError> {
        let target = swf_pcm_rate(sample_rate);
        if matches!(sample_rate, 5_512 | 5_500 | 11_025 | 22_050 | 44_100) {
            return self.define_sound_pcm(samples, channels, sample_rate);
        }
        let resampled = resample_pcm(samples, channels, sample_rate, target)?;
        self.define_sound_pcm(&resampled, channels, target)
    }

    /// Defines a sprite (movie clip) with its own timeline; an identical sprite returns the earlier id.
    ///
    /// # Errors
    /// [`SwfError::TooManyFrames`] past 65535 frames and any encoding error of the items.
    pub fn define_sprite(&mut self, frames: &[Frame]) -> Result<CharacterId, SwfError> {
        let count = u16::try_from(frames.len()).map_err(|_| SwfError::TooManyFrames)?;
        let mut content = Vec::new();
        content.put_u16(count);
        for frame in frames {
            encode_frame(&mut content, frame)?;
        }
        write_tag(&mut content, code::END, &[])?;
        self.define_once(Dedup::Sprite, code::DEFINE_SPRITE, content)
    }

    /// Names a character for `attachMovie` style access (`ExportAssets`).
    pub fn export_asset(&mut self, id: CharacterId, name: impl Into<String>) {
        self.exports.push((id, name.into()));
    }

    /// Starts a new scene at the next frame to be added.
    pub fn begin_scene(&mut self, name: impl Into<String>) {
        self.scenes.push((self.frames, name.into()));
    }

    /// Appends a frame to the main timeline.
    ///
    /// # Errors
    /// [`SwfError::TooManyFrames`] past 65535 frames and any encoding error of the items.
    pub fn add_frame(&mut self, frame: &Frame) -> Result<(), SwfError> {
        if self.frames >= u32::from(u16::MAX) {
            return Err(SwfError::TooManyFrames);
        }
        encode_frame(&mut self.body, frame)?;
        if let Some(label) = &frame.label {
            self.labels.push((self.frames, label.clone()));
        }
        self.frames = self.frames.saturating_add(1);
        Ok(())
    }

    /// Appends an arbitrary definition tag (it must not show a frame).
    ///
    /// # Errors
    /// [`SwfError::TooLarge`] for bodies over 4 GiB.
    pub fn raw_tag(&mut self, tag_code: u16, body: &[u8]) -> Result<(), SwfError> {
        write_tag(&mut self.body, tag_code, body)
    }

    /// Assembles the file.
    ///
    /// # Errors
    /// [`SwfError::TooLarge`] when the file exceeds 4 GiB, [`SwfError::Compression`] if zlib fails.
    pub fn finish(self) -> Result<Vec<u8>, SwfError> {
        let settings = &self.settings;
        let frames = u16::try_from(self.frames).map_err(|_| SwfError::TooManyFrames)?;

        let mut inner = Vec::with_capacity(self.body.len().saturating_add(256));
        Rect::new(0, to_twips(settings.width), 0, to_twips(settings.height)).write(&mut inner);
        inner.put_u16(u16::try_from(num::round_f64_to_u32(settings.frame_rate * 256.0)).unwrap_or(u16::MAX));
        inner.put_u16(frames);

        // FileAttributes must come first.
        let attributes = u8::from(settings.metadata.is_some()).wrapping_shl(4) | u8::from(settings.use_network);
        write_tag(&mut inner, code::FILE_ATTRIBUTES, &[attributes, 0, 0, 0])?;
        if let Some(xmp) = &settings.metadata {
            let mut body = Vec::new();
            body.put_cstr(xmp)?;
            write_tag(&mut inner, code::METADATA, &body)?;
        }
        let mut background = Vec::with_capacity(3);
        put_rgb(&mut background, settings.background);
        write_tag(&mut inner, code::SET_BACKGROUND_COLOR, &background)?;
        if !self.scenes.is_empty() {
            let mut body = Vec::new();
            body.put_encoded_u32(u32::try_from(self.scenes.len()).unwrap_or(u32::MAX));
            for (frame, name) in &self.scenes {
                body.put_encoded_u32(*frame);
                body.put_cstr(name)?;
            }
            body.put_encoded_u32(u32::try_from(self.labels.len()).unwrap_or(u32::MAX));
            for (frame, label) in &self.labels {
                body.put_encoded_u32(*frame);
                body.put_cstr(label)?;
            }
            write_tag(&mut inner, code::DEFINE_SCENE_AND_FRAME_LABEL_DATA, &body)?;
        }
        if !self.exports.is_empty() {
            let mut body = Vec::new();
            body.put_u16(u16::try_from(self.exports.len()).unwrap_or(u16::MAX));
            for (id, name) in self.exports.iter().take(usize::from(u16::MAX)) {
                body.put_u16(id.0);
                body.put_cstr(name)?;
            }
            write_tag(&mut inner, code::EXPORT_ASSETS, &body)?;
        }
        inner.extend_from_slice(&self.body);
        write_tag(&mut inner, code::END, &[])?;

        let total = inner.len().saturating_add(8);
        let length = u32::try_from(total).map_err(|_| SwfError::TooLarge(total))?;
        let mut file = Vec::with_capacity(total);
        file.extend_from_slice(match settings.compression {
            Compression::None => b"FWS",
            Compression::Zlib => b"CWS",
        });
        file.put_u8(settings.version);
        file.put_u32(length);
        match settings.compression {
            Compression::None => file.extend_from_slice(&inner),
            Compression::Zlib => file.extend_from_slice(&zlib(&inner)?),
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use anim_core::kurbo::{Rect as KRect, Shape};
    use anim_display::FillRule;

    use super::*;
    use crate::shape::filled_path_shape;
    use crate::style::FillStyle;
    use crate::tag::scan_tags;
    use crate::types::Matrix;

    fn square_shape() -> ShapeDef {
        filled_path_shape(&KRect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1), FillStyle::Solid(Rgba8::opaque(255, 0, 0)), FillRule::NonZero).unwrap()
    }

    /// Splits an uncompressed file into (version, declared length, tags after the frame header).
    fn tags_of(file: &[u8]) -> Vec<(u16, Vec<u8>)> {
        assert_eq!(&file[..3], b"FWS");
        let declared = u32::from_le_bytes(file[4..8].try_into().unwrap());
        assert_eq!(usize::try_from(declared).unwrap(), file.len());
        // RECT: 5 bit count then 4 fields
        let nbits = usize::from(file[8] >> 3);
        let rect_bytes = 4_usize.saturating_mul(nbits).saturating_add(5).div_ceil(8);
        let rest = &file[rect_bytes.saturating_add(12)..];
        scan_tags(rest).into_iter().map(|t| (t.code, t.body.to_vec())).collect()
    }

    fn uncompressed() -> MovieSettings {
        MovieSettings { compression: Compression::None, ..MovieSettings::default() }
    }

    #[test]
    fn empty_movie_has_header_tags_and_end() {
        let file = Movie::new(uncompressed()).unwrap().finish().unwrap();
        let codes: Vec<u16> = tags_of(&file).iter().map(|t| t.0).collect();
        assert_eq!(codes, vec![code::FILE_ATTRIBUTES, code::SET_BACKGROUND_COLOR, code::END]);
    }

    #[test]
    fn identical_shapes_share_a_character_and_sprites_nest() {
        let mut movie = Movie::new(uncompressed()).unwrap();
        let a = movie.define_shape(&square_shape()).unwrap();
        let b = movie.define_shape(&square_shape()).unwrap();
        assert_eq!(a, b);
        let sprite = movie.define_sprite(&[Frame::new(vec![FrameItem::Place(Placement::new(1, a, Matrix::IDENTITY))])]).unwrap();
        assert_ne!(sprite, a);
        let again = movie.define_sprite(&[Frame::new(vec![FrameItem::Place(Placement::new(1, a, Matrix::IDENTITY))])]).unwrap();
        assert_eq!(sprite, again);
        movie.add_frame(&Frame::new(vec![FrameItem::Place(Placement::new(1, sprite, Matrix::IDENTITY))])).unwrap();
        assert_eq!(movie.frame_count(), 1);
        assert_eq!(movie.character_count(), 2);
        let file = movie.finish().unwrap();
        let codes: Vec<u16> = tags_of(&file).iter().map(|t| t.0).collect();
        assert_eq!(
            codes,
            vec![code::FILE_ATTRIBUTES, code::SET_BACKGROUND_COLOR, code::DEFINE_SHAPE4, code::DEFINE_SPRITE, code::PLACE_OBJECT3, code::SHOW_FRAME, code::END]
        );
    }

    #[test]
    fn compression_wraps_everything_after_the_first_eight_bytes() {
        let mut movie = Movie::new(MovieSettings::default()).unwrap();
        movie.define_shape(&square_shape()).unwrap();
        let file = movie.finish().unwrap();
        assert_eq!(&file[..3], b"CWS");
        assert_eq!(file[3], DEFAULT_VERSION);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        assert!(Movie::new(MovieSettings { width: 0.0, ..MovieSettings::default() }).is_err());
        assert!(Movie::new(MovieSettings { height: f64::NAN, ..MovieSettings::default() }).is_err());
        assert!(Movie::new(MovieSettings { frame_rate: 0.0, ..MovieSettings::default() }).is_err());
        assert!(Movie::new(MovieSettings { frame_rate: 300.0, ..MovieSettings::default() }).is_err());
        assert_eq!(Movie::new(MovieSettings { version: 4, ..MovieSettings::default() }).unwrap().settings().version, MIN_VERSION);
    }

    #[test]
    fn xmp_packets_escape_the_title() {
        let xmp = xmp_packet("Tom & <Jerry> \"1\"");
        assert!(xmp.contains("Tom &amp; &lt;Jerry&gt; &quot;1&quot;"));
        assert!(xmp.starts_with("<?xpacket begin="));
        assert!(xmp.ends_with("<?xpacket end=\"w\"?>"));
        let file = Movie::new(MovieSettings { metadata: Some(xmp), compression: Compression::None, ..MovieSettings::default() }).unwrap().finish().unwrap();
        let codes: Vec<u16> = tags_of(&file).iter().map(|t| t.0).collect();
        assert_eq!(&codes[..3], &[code::FILE_ATTRIBUTES, code::METADATA, code::SET_BACKGROUND_COLOR]);
    }

    #[test]
    fn scenes_and_labels_are_recorded() {
        let mut movie = Movie::new(uncompressed()).unwrap();
        movie.begin_scene("Scene 1");
        movie.add_frame(&Frame { label: Some("intro".into()), items: vec![] }).unwrap();
        movie.begin_scene("Scene 2");
        movie.add_frame(&Frame { label: Some("outro".into()), items: vec![FrameItem::DoAction(ACTION_STOP.to_vec())] }).unwrap();
        let file = movie.finish().unwrap();
        let tags = tags_of(&file);
        let scene = tags.iter().find(|t| t.0 == code::DEFINE_SCENE_AND_FRAME_LABEL_DATA).unwrap();
        assert_eq!(scene.1, b"\x02\x00Scene 1\0\x01Scene 2\0\x02\x00intro\0\x01outro\0".to_vec());
        assert!(tags.iter().any(|t| t.0 == code::DO_ACTION && t.1 == ACTION_STOP));
    }
}
