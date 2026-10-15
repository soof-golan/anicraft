//! The soundtrack of a scene: sounds attached to keyframes, mixed into one buffer for video export.
//!
//! Rules (Animate's):
//! * Event, Start and Stream sounds begin at their keyframe and play to their end, repeated by the loop setting.
//!   A looping-forever sound plays to the end of the exported range.
//! * A Stop cue ends every earlier sound of the same name at its frame.
//! * A sound that starts before the exported range is cut, so a range exports what a player would have heard.
//!
//! Not modelled: envelopes and effects (fades, pan), and sounds inside symbols.

use std::collections::BTreeMap;
use std::ops::Range;

use anim_audio::{MixFormat, MixInput, PcmBuffer, decode_flash_sound, mix};
use anim_core::num;
use anim_scene::{Document, FrameSound, MediaStore, SoundLoop, SoundSync, Timeline};

use crate::error::Result;

/// Most times one cue repeats, whatever the file says.
const MAX_REPEATS: u32 = 10_000;

/// The sample rate of exported soundtracks.
pub const SAMPLE_RATE: u32 = 48_000;

/// One sound starting at a keyframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    /// Library name of the sound.
    pub name: String,
    /// 0-based frame it starts at.
    pub frame: u32,
    /// Event, start, stop or stream.
    pub sync: SoundSync,
    /// How often it repeats.
    pub looping: SoundLoop,
}

/// The soundtrack and what could not be used.
#[derive(Debug, Default)]
pub struct Soundtrack {
    /// Stereo mix for the whole span, `None` when the scene has no sound.
    pub track: Option<PcmBuffer>,
    /// Sounds that were missing or could not be decoded.
    pub notes: Vec<String>,
}

/// Every sound cue of a scene, in frame order.
#[must_use]
pub fn cues(scene: &Timeline) -> Vec<Cue> {
    let mut cues: Vec<Cue> =
        scene.layers.iter().flat_map(|layer| layer.frames.iter()).filter_map(|frame| frame.sound.as_ref().map(|sound| cue_of(frame.index, sound))).collect();
    cues.sort_by_key(|c| c.frame);
    cues
}

fn cue_of(frame: u32, sound: &FrameSound) -> Cue {
    Cue { name: sound.name.clone(), frame, sync: sound.sync, looping: sound.loop_mode }
}

/// Mixes the sounds of `scene` for the frames in `span`.
///
/// # Errors
/// [`crate::error::CliError::Audio`] when mixing fails; sounds that cannot be decoded are skipped and reported in
/// [`Soundtrack::notes`].
pub fn soundtrack(doc: &Document, media: &MediaStore, scene: usize, span: &Range<u32>) -> Result<Soundtrack> {
    let mut out = Soundtrack::default();
    let Some(timeline) = doc.scenes.get(scene) else { return Ok(out) };
    let all = cues(timeline);
    if all.is_empty() {
        return Ok(out);
    }
    let rate = doc.frame_rate_exact();
    let span_len = i64::from(span.end).saturating_sub(i64::from(span.start));
    let length = rate.tick_of(span_len);
    let mut decoded: BTreeMap<String, Option<PcmBuffer>> = BTreeMap::new();
    let mut placed: Vec<(PcmBuffer, i64)> = Vec::new();
    for cue in all.iter().filter(|c| c.sync != SoundSync::Stop && c.frame < span.end) {
        let buffer = decoded.entry(cue.name.clone()).or_insert_with(|| load_sound(doc, media, &cue.name, &mut out.notes));
        let Some(buffer) = buffer.as_ref().filter(|b| !b.is_empty()) else { continue };
        let start_frame = i64::from(cue.frame).saturating_sub(i64::from(span.start));
        // The sound stops at the next Stop cue for it, or at the end of the span.
        let stop_frame =
            all.iter().find(|c| c.sync == SoundSync::Stop && c.name == cue.name && c.frame > cue.frame).map_or(span.end, |c| c.frame.min(span.end));
        let limit_frames = frames_between(doc, start_frame, i64::from(stop_frame).saturating_sub(i64::from(span.start)), buffer.sample_rate);
        let mut sound = repeat(buffer, cue.looping, limit_frames);
        sound.samples.truncate(limit_frames.saturating_mul(usize::from(sound.channels)));
        placed.push((sound, start_frame));
    }
    if placed.is_empty() {
        return Ok(out);
    }
    let inputs: Vec<MixInput<'_>> = placed.iter().map(|(buf, frame)| MixInput::at(buf, rate.tick_of(*frame))).collect();
    let mut mixed = mix(&inputs, MixFormat { sample_rate: SAMPLE_RATE, channels: 2 }, Some(length))?;
    mixed.sanitize();
    for s in &mut mixed.samples {
        *s = s.clamp(-1.0, 1.0);
    }
    out.track = Some(mixed);
    Ok(out)
}

/// Sample frames between two document frames at `sample_rate`.
fn frames_between(doc: &Document, from: i64, to: i64, sample_rate: u32) -> usize {
    let seconds = doc.frame_rate_exact().tick_of(to.saturating_sub(from)).seconds().max(0.0);
    num::round_f64_to_u32(seconds * f64::from(sample_rate)).try_into().unwrap_or(usize::MAX)
}

fn load_sound(doc: &Document, media: &MediaStore, name: &str, notes: &mut Vec<String>) -> Option<PcmBuffer> {
    let Some(item) = doc.library.find_sound(name) else {
        notes.push(format!("sound {name:?} is not in the library"));
        return None;
    };
    let Some(bytes) = anicraft_xfl::sound_payload(media, item) else {
        notes.push(format!("sound {name:?} has no audio data"));
        return None;
    };
    match decode_flash_sound(bytes, Some(name), item.format.as_deref()) {
        Ok(buffer) => Some(buffer),
        Err(e) => {
            notes.push(format!("sound {name:?} could not be decoded: {e}"));
            None
        }
    }
}

/// `buffer` repeated as `looping` says, at most `limit` sample frames long.
fn repeat(buffer: &PcmBuffer, looping: SoundLoop, limit: usize) -> PcmBuffer {
    let times = match looping {
        SoundLoop::Once => 1,
        SoundLoop::Repeat(n) => n.clamp(1, MAX_REPEATS),
        SoundLoop::Forever => {
            let each = buffer.frames().max(1);
            u32::try_from(limit.div_ceil(each)).unwrap_or(MAX_REPEATS).clamp(1, MAX_REPEATS)
        }
    };
    let mut out = buffer.clone();
    for _ in 1..times {
        if out.frames() >= limit || !out.append(buffer) {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use anim_scene::{Frame, Layer};

    use super::*;

    fn scene_with(sounds: &[(u32, &str, SoundSync)]) -> Timeline {
        let mut layer = Layer::new("sound", anim_core::Rgba8::opaque(0, 0, 0));
        for (index, name, sync) in sounds {
            let mut frame = Frame::new(*index, 1, Vec::new());
            let mut sound = FrameSound::new(*name);
            sound.sync = *sync;
            frame.sound = Some(sound);
            layer.frames.push(frame);
        }
        let mut timeline = Timeline::new("Scene 1");
        timeline.layers.push(layer);
        timeline
    }

    #[test]
    fn cues_come_in_frame_order() {
        let tl = scene_with(&[(8, "b", SoundSync::Stop), (2, "a", SoundSync::Start)]);
        let cues = cues(&tl);
        assert_eq!(cues.iter().map(|c| (c.frame, c.name.as_str())).collect::<Vec<_>>(), vec![(2, "a"), (8, "b")]);
    }

    #[test]
    fn looping_forever_fills_the_limit() {
        let one = PcmBuffer::new(1000, 1, vec![0.5; 100]);
        assert_eq!(repeat(&one, SoundLoop::Once, 1000).frames(), 100);
        assert_eq!(repeat(&one, SoundLoop::Repeat(3), 1000).frames(), 300);
        assert!(repeat(&one, SoundLoop::Forever, 1000).frames() >= 1000);
    }

    #[test]
    fn a_missing_sound_is_reported_not_fatal() {
        let doc = Document { scenes: vec![scene_with(&[(0, "ghost", SoundSync::Event)])], ..Document::default() };
        let result = soundtrack(&doc, &MediaStore::new(), 0, &(0..24)).unwrap();
        assert!(result.track.is_none());
        assert_eq!(result.notes.len(), 1);
    }
}
