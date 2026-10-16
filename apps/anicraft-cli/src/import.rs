//! Importing artwork into the library: the code behind File > Import and the `library.import` command.

use std::path::Path;

use anim_core::{Affine, num};
use anim_scene::{BitmapCompression, BitmapInstance, BitmapItem, Element, LibraryItem, SoundItem};

use crate::mcp::AniDoc;

/// Picks a library name that is free: `name`, then `name 2`, `name 3`, ...
fn free_name(doc: &anim_scene::Document, wanted: &str) -> String {
    if doc.library.find_index(wanted).is_none() {
        return wanted.to_owned();
    }
    let (stem, ext) = wanted.rsplit_once('.').map_or((wanted, ""), |(s, e)| (s, e));
    for n in 2..10_000 {
        let candidate = if ext.is_empty() { format!("{stem} {n}") } else { format!("{stem} {n}.{ext}") };
        if doc.library.find_index(&candidate).is_none() {
            return candidate;
        }
    }
    wanted.to_owned()
}

/// What an import produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The library name of the new bitmap.
    pub name: String,
    /// Its size in pixels.
    pub size: [f64; 2],
}

/// Imports artwork into the library (and onto the stage when `to_stage`). Returns the library name.
///
/// Raster images are stored as they are. PSD, AI/PDF and SVG files are rasterised at their own size (their layers are
/// flattened); the editor keeps no vector import yet. `name` overrides the library name (a free variant is chosen when
/// it is taken).
///
/// # Errors
/// A message for unreadable or unsupported files.
pub fn import_artwork_as(doc: &mut AniDoc, path: &Path, name: Option<&str>, to_stage: bool) -> Result<Imported, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_name = path.file_name().map_or_else(|| "image.png".to_owned(), |n| n.to_string_lossy().into_owned());
    if is_audio(&bytes) {
        return import_sound(doc, &bytes, &file_name, name);
    }
    let (data, default_name, compression, size) = if anim_artwork::detect(&bytes).is_some() {
        let tree = anim_artwork::import(&bytes, &anim_artwork::Limits::default()).map_err(|e| format!("{file_name}: {e}"))?;
        let mut store = anim_display::ImageStore::new();
        let list = anim_artwork::to_display(&tree, &mut store);
        let bounds = tree.root.bounds().unwrap_or_else(|| anim_core::Rect::new(0.0, 0.0, list.size.width.max(1.0), list.size.height.max(1.0)));
        let width = u32::try_from(num::ceil_f64_to_i32(bounds.width().max(1.0))).unwrap_or(1).clamp(1, 8192);
        let height = u32::try_from(num::ceil_f64_to_i32(bounds.height().max(1.0))).unwrap_or(1).clamp(1, 8192);
        let options = anim_render::RenderOptions::new(width, height)
            .with_view(Affine::translate((-bounds.x0, -bounds.y0)))
            .with_background(anim_render::Background::Transparent);
        let image = anim_render::render(&list, &store, &options).map_err(|e| e.to_string())?;
        let png = anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image)).map_err(|e| e.to_string())?;
        let stem = path.file_stem().map_or_else(|| "artwork".to_owned(), |s| s.to_string_lossy().into_owned());
        (png, format!("{stem}.png"), BitmapCompression::Lossless, [f64::from(image.width), f64::from(image.height)])
    } else {
        let image = anicraft_xfl::decode_bitmap_bytes(&bytes).map_err(|e| format!("{file_name}: {e}"))?;
        let jpeg = bytes.starts_with(&[0xFF, 0xD8]);
        (bytes, file_name.clone(), if jpeg { BitmapCompression::Photo } else { BitmapCompression::Lossless }, [f64::from(image.width), f64::from(image.height)])
    };
    let wanted = name.filter(|n| !n.is_empty()).unwrap_or(&default_name);
    let name = free_name(doc.document(), wanted);
    let media = doc.media.insert(data).map_err(|e| e.to_string())?;
    let item_id = doc.document().library.fresh_item_id(&name);
    let mut item = BitmapItem::new(&name, item_id);
    item.href = Some(file_name);
    item.media = Some(media);
    item.compression = compression;
    item.size = Some(size);
    doc.editor.document.library.push(LibraryItem::Bitmap(item)).map_err(|e| e.to_string())?;
    if to_stage {
        place_bitmap(doc, &name, size)?;
    }
    Ok(Imported { name, size })
}

/// True for the containers `anim-audio` can read (WAV, MP3, Ogg, FLAC), judged by their first bytes.
fn is_audio(bytes: &[u8]) -> bool {
    let wave = bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE");
    let mp3 = bytes.starts_with(b"ID3") || matches!(bytes.get(..2), Some([0xFF, 0xFB | 0xF3 | 0xF2]));
    wave || mp3 || bytes.starts_with(b"OggS") || bytes.starts_with(b"fLaC")
}

/// Sample rates Animate's sound items use, with the name it gives them.
const SOUND_RATES: [(u32, &str); 4] = [(5512, "5kHz"), (11_025, "11kHz"), (22_050, "22kHz"), (44_100, "44kHz")];

/// Imports a sound the way Animate stores one: 16 bit little-endian PCM at 5, 11, 22 or 44 kHz, mono or stereo, with
/// the `format` text and sample count of the item. MP3 stays MP3 (the player decodes it from the container).
fn import_sound(doc: &mut AniDoc, bytes: &[u8], file_name: &str, name: Option<&str>) -> Result<Imported, String> {
    let pcm = anim_audio::decode_bytes(bytes, Some(file_name)).map_err(|e| format!("{file_name}: {e}"))?;
    if pcm.is_empty() {
        return Err(format!("{file_name}: the sound is empty"));
    }
    let wave = bytes.starts_with(b"RIFF");
    let (data, format, frames) = if wave {
        // The nearest Animate rate, resampling when the file has another one.
        let (rate, label) = SOUND_RATES.iter().copied().min_by_key(|(r, _)| r.abs_diff(pcm.sample_rate)).unwrap_or((44_100, "44kHz"));
        let channels = if pcm.channels >= 2 { 2 } else { 1 };
        let converted = anim_audio::resample(&pcm.remix(channels), rate).map_err(|e| format!("{file_name}: {e}"))?;
        let data: Vec<u8> = converted
            .samples
            .iter()
            .flat_map(|s| i16::try_from(num::round_f64_to_i32(f64::from(s.clamp(-1.0, 1.0)) * 32767.0)).unwrap_or(0).to_le_bytes())
            .collect();
        let layout = if channels == 2 { "Stereo" } else { "Mono" };
        (data, format!("{label} 16bit {layout}"), converted.frames())
    } else {
        let label = SOUND_RATES.iter().copied().min_by_key(|(r, _)| r.abs_diff(pcm.sample_rate)).map_or("44kHz", |(_, l)| l);
        (bytes.to_vec(), format!("{label} {} MP3", if pcm.channels >= 2 { "Stereo" } else { "Mono" }), pcm.frames())
    };
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    let wanted = name.filter(|n| !n.is_empty()).map_or_else(|| format!("{stem}{}", if wave { ".wav" } else { ".mp3" }), str::to_owned);
    let name = free_name(doc.document(), &wanted);
    let media = doc.media.insert(data).map_err(|e| e.to_string())?;
    let item_id = doc.document().library.fresh_item_id(&name);
    let mut item = SoundItem::new(&name, item_id);
    item.href = Some(file_name.to_owned());
    item.media = Some(media);
    item.format = Some(format);
    item.sample_count = Some(u64::try_from(frames).unwrap_or(u64::MAX));
    doc.editor.document.library.push(LibraryItem::Sound(item)).map_err(|e| e.to_string())?;
    Ok(Imported { name, size: [0.0, 0.0] })
}

/// [`import_artwork_as`] with the default library name; returns just the name.
///
/// # Errors
/// A message for unreadable or unsupported files.
pub fn import_artwork(doc: &mut AniDoc, path: &Path, to_stage: bool) -> Result<String, String> {
    import_artwork_as(doc, path, None, to_stage).map(|i| i.name)
}

/// Puts a bitmap instance centred on the stage on the current layer and frame.
///
/// # Errors
/// When the layer or frame does not exist.
pub fn place_bitmap(doc: &mut AniDoc, name: &str, size: [f64; 2]) -> Result<(), String> {
    let id = doc.editor.state.timeline.clone();
    let (layer, frame) = (doc.editor.state.layer, doc.editor.state.frame);
    let (w, h) = (doc.document().width, doc.document().height);
    let matrix = Affine::translate(((w - size[0]) / 2.0, (h - size[1]) / 2.0));
    let at = anim_scene::edit::add_element(&mut doc.editor.document, &id, layer, frame, Element::Bitmap(BitmapInstance::new(name, matrix)))
        .map_err(|e| e.to_string())?;
    let keyframe = doc.document().timeline(&id).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map_or(0, |k| k.index);
    doc.editor.state.selection = anicraft_tools::Selection::one(anicraft_tools::ElementRef::top_level(layer, keyframe, at));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16 bit PCM WAV of a decaying sine, `seconds` long.
    fn wav(rate: u32, channels: u16, seconds: f64) -> Vec<u8> {
        let frames = num::round_f64_to_u32(f64::from(rate) * seconds);
        let mut data = Vec::new();
        for i in 0..frames {
            let t = f64::from(i) / f64::from(rate);
            let level = (t * 440.0 * std::f64::consts::TAU).sin() * (1.0 - t / seconds) * 20_000.0;
            let sample = i16::try_from(num::round_f64_to_i32(level)).unwrap();
            for _ in 0..channels {
                data.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().saturating_add(36).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&rate.saturating_mul(u32::from(channels)).saturating_mul(2).to_le_bytes());
        out.extend_from_slice(&channels.saturating_mul(2).to_le_bytes());
        out.extend_from_slice(&16_u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn a_wav_becomes_a_sound_item_in_animates_pcm_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        std::fs::write(&path, wav(22_050, 1, 0.5)).unwrap();
        let mut doc = AniDoc::default();
        let imported = import_artwork_as(&mut doc, &path, None, false).unwrap();
        assert_eq!(imported.name, "tone.wav");
        let item = doc.document().library.find_sound("tone.wav").unwrap();
        assert_eq!(item.format.as_deref(), Some("22kHz 16bit Mono"));
        assert_eq!(item.sample_count, Some(11_025));
        let payload = anicraft_xfl::sound_payload(&doc.media, item).unwrap();
        assert_eq!(payload.len(), 11_025 * 2, "raw 16 bit samples, no RIFF header");
        // The raw layout decodes back through the format text, like a sound read from a file.
        let back = anim_audio::decode_flash_sound(payload, Some("tone.wav"), item.format.as_deref()).unwrap();
        assert_eq!((back.sample_rate, back.channels, back.frames()), (22_050, 1, 11_025));
        // A second import gets a free name.
        assert_eq!(import_artwork(&mut doc, &path, false).unwrap(), "tone 2.wav");
    }

    #[test]
    fn other_rates_and_channel_counts_are_brought_to_animates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.wav");
        std::fs::write(&path, wav(48_000, 2, 0.25)).unwrap();
        let mut doc = AniDoc::default();
        import_artwork(&mut doc, &path, false).unwrap();
        let item = doc.document().library.find_sound("wide.wav").unwrap();
        assert_eq!(item.format.as_deref(), Some("44kHz 16bit Stereo"));
        let seconds = item.sample_count.map(|n| f64::from(u32::try_from(n).unwrap()) / 44_100.0).unwrap();
        assert!((seconds - 0.25).abs() < 0.01, "{seconds}");
    }

    #[test]
    fn a_broken_wav_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.wav");
        std::fs::write(&path, b"RIFF\0\0\0\0WAVEjunk").unwrap();
        let mut doc = AniDoc::default();
        assert!(import_artwork(&mut doc, &path, false).is_err());
        assert_eq!(doc.document().library.items.len(), 0);
    }
}
