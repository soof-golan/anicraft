//! The Library panel's preview: what the selected item looks (or sounds) like.
//!
//! Symbols are drawn by the render worker at the frame the preview is on, bitmaps are decoded and scaled down, sounds
//! are decoded and reduced to waveform peaks. Everything here is either a pure function (captions, which frame a
//! running preview is on) or runs on the worker; the window keeps the result in [`PreviewState`].

use std::time::{Duration, Instant};

use anim_audio::{PcmBuffer, waveform_peaks};
use anim_core::{Affine, num};
use anim_display::Image;
use anim_render::{Background, RenderOptions};
use anim_scene::{Document, LibraryItem, MediaStore, SymbolKind};
use anim_timeline::{EvalOptions, Evaluator};
use anim_ui::library::LibraryPreview;
use egui::TextureHandle;

/// Pixels of the square the worker renders a preview into; the panel scales it down to fit.
pub const PREVIEW_SIDE: u32 = 320;

/// Bars in a sound's waveform.
const WAVE_BINS: usize = 96;

/// What kind of thing an item previews as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A symbol that can play: more than one frame, not a button.
    Clip {
        frames: u32,
    },
    /// A symbol drawn at its first frame (single frame, or a button).
    Still,
    Bitmap,
    Sound,
    /// Folders, fonts and videos have no picture; the panel shows their icon and a description.
    Other,
}

/// Classifies library item `name`.
#[must_use]
pub fn kind_of(doc: &Document, name: &str) -> Kind {
    match doc.library.find(name) {
        Some(LibraryItem::Symbol(s)) => {
            let frames = s.timeline.frame_count().max(1);
            if frames > 1 && s.kind != SymbolKind::Button { Kind::Clip { frames } } else { Kind::Still }
        }
        Some(LibraryItem::Bitmap(_)) => Kind::Bitmap,
        Some(LibraryItem::Sound(_)) => Kind::Sound,
        _ => Kind::Other,
    }
}

/// What the worker found for a preview request.
#[derive(Debug)]
pub enum PreviewData {
    /// Pixels, premultiplied, and the size of the content in document pixels when known.
    Picture { image: Image, content: Option<[f64; 2]> },
    /// A decoded sound and its waveform (0..=255 per bar, normalised to the loudest).
    Sound { peaks: Vec<u8>, pcm: PcmBuffer },
    /// Nothing to show; the text says why.
    Nothing(String),
}

/// Renders the preview of `name` at `frame`.
///
/// `playing` lets nested movie clips advance with the frame, as they do in a published movie.
#[must_use]
pub fn render_preview(ev: &mut Evaluator<'_>, media: &MediaStore, name: &str, frame: u32, playing: bool) -> PreviewData {
    let doc = ev.document();
    match doc.library.find(name) {
        Some(LibraryItem::Symbol(_)) => symbol_picture(ev, name, frame, playing),
        Some(LibraryItem::Bitmap(item)) => {
            let Some(id) = item.media else { return PreviewData::Nothing("no pixel data".to_owned()) };
            match anicraft_xfl::decode_bitmap(media, id) {
                Ok(image) => {
                    let content = Some([f64::from(image.width), f64::from(image.height)]);
                    PreviewData::Picture { image: shrink(&image, PREVIEW_SIDE), content }
                }
                Err(e) => PreviewData::Nothing(format!("cannot decode: {e}")),
            }
        }
        Some(LibraryItem::Sound(item)) => {
            let Some(bytes) = anicraft_xfl::sound_payload(media, item) else { return PreviewData::Nothing("no audio data".to_owned()) };
            match anim_audio::decode_flash_sound(bytes, Some(name), item.format.as_deref()) {
                Ok(pcm) if !pcm.is_empty() => PreviewData::Sound { peaks: normalised_peaks(&pcm), pcm },
                Ok(_) => PreviewData::Nothing("the sound is empty".to_owned()),
                Err(e) => PreviewData::Nothing(format!("cannot decode: {e}")),
            }
        }
        _ => PreviewData::Nothing(String::new()),
    }
}

/// The symbol's frame, fitted into the preview square on the stage colour.
fn symbol_picture(ev: &mut Evaluator<'_>, name: &str, frame: u32, playing: bool) -> PreviewData {
    ev.set_options(EvalOptions { advance_movie_clips: playing, ..*ev.options() });
    // Fit to the first frame's bounds so the picture does not jump around while it plays; a symbol whose first
    // frame is empty fits to the frame being shown.
    let first = ev.symbol_bounds(name, 0).filter(|b| b.width() > 0.0 && b.height() > 0.0);
    let bounds = first.or_else(|| ev.symbol_bounds(name, frame).filter(|b| b.width() > 0.0 && b.height() > 0.0));
    let background = ev.document().background_color;
    let side = PREVIEW_SIDE;
    let mut canvas = crate::worker::solid(side, side, Some(background));
    let Some(bounds) = bounds else { return PreviewData::Picture { image: canvas, content: None } };
    let list = ev.frame_of_symbol(name, frame);
    let inner = f64::from(side) - 16.0;
    let scale = (inner / bounds.width()).min(inner / bounds.height());
    let view =
        Affine::translate((f64::from(side) / 2.0, f64::from(side) / 2.0)) * Affine::scale(scale) * Affine::translate((-bounds.center().x, -bounds.center().y));
    let options = RenderOptions::new(side, side).with_view(view).with_background(Background::Transparent);
    match anim_render::render(&list, ev.images(), &options) {
        Ok(layer) => {
            crate::worker::over(&mut canvas, &layer);
            PreviewData::Picture { image: canvas, content: Some([bounds.width(), bounds.height()]) }
        }
        Err(e) => PreviewData::Nothing(format!("cannot draw: {e}")),
    }
}

/// Waveform bars scaled so the loudest one is full height (a quiet sound is still readable).
fn normalised_peaks(pcm: &PcmBuffer) -> Vec<u8> {
    let peaks = waveform_peaks(pcm, WAVE_BINS);
    let loudest = peaks.iter().map(|p| p.magnitude()).fold(0.0_f32, f32::max);
    peaks
        .iter()
        .map(|p| {
            let level = if loudest > 0.0 { p.magnitude() / loudest } else { 0.0 };
            u8::try_from(num::round_f64_to_u32(f64::from(level.clamp(0.0, 1.0)) * 255.0)).unwrap_or(255)
        })
        .collect()
}

/// `image` reduced by whole box averages so its longer side is at most `max_side`.
#[must_use]
pub fn shrink(image: &Image, max_side: u32) -> Image {
    let longest = image.width.max(image.height);
    if longest <= max_side || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let ratio = f64::from(max_side) / f64::from(longest);
    let out_w = num::round_f64_to_u32(f64::from(image.width) * ratio).max(1);
    let out_h = num::round_f64_to_u32(f64::from(image.height) * ratio).max(1);
    let (src_w, src_h) = (num::u32_to_usize(image.width), num::u32_to_usize(image.height));
    let mut data = Vec::with_capacity(num::u32_to_usize(out_w).saturating_mul(num::u32_to_usize(out_h)).saturating_mul(4));
    for oy in 0..out_h {
        let y0 = num::u32_to_usize(oy).saturating_mul(src_h).checked_div(num::u32_to_usize(out_h)).unwrap_or(0);
        let y1 = num::u32_to_usize(oy.saturating_add(1))
            .saturating_mul(src_h)
            .checked_div(num::u32_to_usize(out_h))
            .unwrap_or(0)
            .max(y0.saturating_add(1))
            .min(src_h);
        for ox in 0..out_w {
            let x0 = num::u32_to_usize(ox).saturating_mul(src_w).checked_div(num::u32_to_usize(out_w)).unwrap_or(0);
            let x1 = num::u32_to_usize(ox.saturating_add(1))
                .saturating_mul(src_w)
                .checked_div(num::u32_to_usize(out_w))
                .unwrap_or(0)
                .max(x0.saturating_add(1))
                .min(src_w);
            let mut sum = [0_u64; 4];
            let mut count = 0_u64;
            for y in y0..y1 {
                let row = y.saturating_mul(src_w).saturating_mul(4);
                let from = row.saturating_add(x0.saturating_mul(4));
                let to = row.saturating_add(x1.saturating_mul(4));
                for px in image.premultiplied_rgba.get(from..to).unwrap_or_default().chunks_exact(4) {
                    for (s, v) in sum.iter_mut().zip(px) {
                        *s = s.saturating_add(u64::from(*v));
                    }
                    count = count.saturating_add(1);
                }
            }
            for s in sum {
                data.push(u8::try_from(s.checked_div(count.max(1)).unwrap_or(0)).unwrap_or(255));
            }
        }
    }
    Image { width: out_w, height: out_h, premultiplied_rgba: data }
}

/// What the Library panel remembers about the preview of the selected item.
#[derive(Default)]
pub struct PreviewState {
    /// The item the state belongs to.
    pub name: Option<String>,
    /// The session revision the picture was requested at; a symbol is redrawn when the document changes.
    pub revision: u64,
    pub texture: Option<TextureHandle>,
    pub texture_size: [u32; 2],
    pub content: Option<[f64; 2]>,
    pub peaks: Vec<u8>,
    pub pcm: Option<PcmBuffer>,
    /// Why there is nothing to show, if so.
    pub note: String,
    /// The frame of the picture last asked for.
    pub requested_frame: Option<u32>,
    /// Playing since this moment (moved back by the offset when playback starts part way, or on a seek).
    pub started: Option<Instant>,
    /// Start playing as soon as the sound has been decoded (screenshots of the playing state).
    pub autoplay: bool,
}

impl PreviewState {
    /// Seconds of a sound.
    #[must_use]
    pub fn sound_seconds(&self) -> f64 {
        self.pcm.as_ref().map_or(0.0, PcmBuffer::seconds)
    }

    /// How long one pass of the preview lasts: the clip's frames at the document rate, or the sound.
    #[must_use]
    pub fn length(&self, kind: Kind, fps: f64) -> Duration {
        let seconds = match kind {
            Kind::Clip { frames } => f64::from(frames) / fps.max(1.0),
            Kind::Sound => self.sound_seconds(),
            _ => 0.0,
        };
        Duration::from_secs_f64(seconds.max(0.0))
    }

    /// 0..=1000 along the preview; a clip wraps around, a sound stops at its end.
    #[must_use]
    pub fn progress_permille(&self, kind: Kind, fps: f64, now: Instant) -> u16 {
        let total = self.length(kind, fps).as_secs_f64();
        let Some(started) = self.started else {
            return 0;
        };
        if total <= 0.0 {
            return 0;
        }
        let elapsed = now.saturating_duration_since(started).as_secs_f64();
        let along = if matches!(kind, Kind::Clip { .. }) { elapsed % total } else { elapsed.min(total) };
        u16::try_from(num::round_f64_to_u32(along / total * 1000.0)).unwrap_or(1000).min(1000)
    }

    /// The frame a running clip is on at `now`.
    #[must_use]
    pub fn clip_frame(&self, frames: u32, fps: f64, now: Instant) -> u32 {
        let Some(started) = self.started else { return 0 };
        let elapsed = now.saturating_duration_since(started).as_secs_f64();
        let frame = num::floor_f64_to_i32(elapsed * fps.max(1.0));
        u32::try_from(frame).unwrap_or(0).checked_rem(frames.max(1)).unwrap_or(0)
    }

    /// Whether a running sound has reached its end.
    #[must_use]
    pub fn sound_finished(&self, now: Instant) -> bool {
        self.started.is_some_and(|s| now.saturating_duration_since(s).as_secs_f64() >= self.sound_seconds())
    }

    /// Moves a running (or stopped) preview to `permille` of its length, as if it had been playing since then.
    pub fn seek(&mut self, kind: Kind, fps: f64, permille: u16, now: Instant) {
        let total = self.length(kind, fps).as_secs_f64();
        let offset = total * f64::from(permille.min(1000)) / 1000.0;
        self.started = Some(now.checked_sub(Duration::from_secs_f64(offset.max(0.0))).unwrap_or(now));
    }
}

/// A short description of item `name`, for the line under the preview.
#[must_use]
pub fn caption(doc: &Document, name: &str, state: &PreviewState) -> String {
    let display = |n: &str| anicraft_xfl::display_name(n).into_owned();
    let size = |content: Option<[f64; 2]>| content.map_or_else(String::new, |[w, h]| format!(", {:.0} x {:.0} px", w.round(), h.round()));
    let text = match doc.library.find(name) {
        Some(LibraryItem::Symbol(s)) => {
            let kind = match s.kind {
                SymbolKind::Graphic => "Graphic",
                SymbolKind::MovieClip => "Movie clip",
                SymbolKind::Button => "Button",
            };
            let frames = s.timeline.frame_count().max(1);
            format!("{kind}, {frames} frame{}{}", if frames == 1 { "" } else { "s" }, size(state.content))
        }
        Some(LibraryItem::Bitmap(b)) => {
            let from_item = b.size;
            format!(
                "Bitmap{}, {}",
                size(state.content.or(from_item)),
                if matches!(b.compression, anim_scene::BitmapCompression::Photo) { "photo" } else { "lossless" }
            )
        }
        Some(LibraryItem::Sound(s)) => {
            let format = s.format.clone().unwrap_or_default();
            let length = if state.pcm.is_some() { format!(", {:.2} s", state.sound_seconds()) } else { String::new() };
            if format.is_empty() { format!("Sound{length}") } else { format!("Sound, {format}{length}") }
        }
        Some(LibraryItem::Font(f)) => format!("Font, {}", f.font.clone().unwrap_or_else(|| display(name))),
        Some(LibraryItem::Video(v)) => {
            let dims = v.width.zip(v.height).map_or_else(String::new, |(w, h)| format!(", {w:.0} x {h:.0} px"));
            let rate = v.fps.map_or_else(String::new, |f| format!(", {f:.2} fps"));
            format!("Video{dims}{rate}")
        }
        Some(LibraryItem::Folder(_)) => {
            let prefix = format!("{name}/");
            let count = doc.library.items.iter().filter(|i| i.name().starts_with(&prefix)).count();
            format!("Folder, {count} item{}", if count == 1 { "" } else { "s" })
        }
        None => display(name),
    };
    if state.note.is_empty() { text } else { format!("{text} ({})", state.note) }
}

/// The panel's model of the preview for item `name`.
#[must_use]
pub fn model(doc: &Document, name: &str, state: &PreviewState, fps: f64, now: Instant) -> LibraryPreview {
    let kind = kind_of(doc, name);
    let playable = match kind {
        Kind::Clip { .. } => true,
        Kind::Sound => state.pcm.is_some(),
        _ => false,
    };
    LibraryPreview {
        texture: state.texture.as_ref().map(TextureHandle::id),
        texture_size: state.texture_size,
        waveform: state.peaks.clone(),
        playable,
        playing: state.started.is_some() && playable,
        progress_permille: state.progress_permille(kind, fps, now),
        caption: caption(doc, name, state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_started(ago: f64) -> PreviewState {
        let now = Instant::now();
        PreviewState { started: now.checked_sub(Duration::from_secs_f64(ago)), ..PreviewState::default() }
    }

    #[test]
    fn a_running_clip_wraps_around_its_frames() {
        let s = state_started(1.0);
        let now = Instant::now();
        // 24 frames at 24 fps last one second: one second in, the clip is back at its start.
        assert!(s.clip_frame(24, 24.0, now) <= 1);
        let s = state_started(0.5);
        let frame = s.clip_frame(24, 24.0, Instant::now());
        assert!((11..=13).contains(&frame), "{frame}");
        assert_eq!(PreviewState::default().clip_frame(24, 24.0, Instant::now()), 0);
    }

    #[test]
    fn progress_wraps_for_clips_and_stops_for_sounds() {
        let now = Instant::now();
        let clip = state_started(1.5);
        let p = clip.progress_permille(Kind::Clip { frames: 24 }, 24.0, now);
        assert!((400..=600).contains(&p), "{p}");
        let sound = PreviewState { pcm: Some(PcmBuffer::new(1000, 1, vec![0.5; 2000])), ..state_started(5.0) };
        assert_eq!(sound.progress_permille(Kind::Sound, 24.0, now), 1000);
        assert!(sound.sound_finished(now));
        assert_eq!(PreviewState::default().progress_permille(Kind::Sound, 24.0, now), 0);
    }

    #[test]
    fn seeking_moves_the_start_back() {
        let now = Instant::now();
        let mut s = PreviewState { pcm: Some(PcmBuffer::new(1000, 1, vec![0.5; 4000])), ..PreviewState::default() };
        s.seek(Kind::Sound, 24.0, 500, now);
        let p = s.progress_permille(Kind::Sound, 24.0, now);
        assert!((499..=501).contains(&p), "{p}");
    }

    #[test]
    fn shrinking_averages_boxes_and_keeps_the_aspect() {
        // A 4 x 2 image, left half white, right half black, shrunk to a 2 wide image.
        let mut px = Vec::new();
        for _ in 0..2 {
            px.extend_from_slice(&[255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
        }
        let image = Image { width: 4, height: 2, premultiplied_rgba: px };
        let small = shrink(&image, 2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.premultiplied_rgba, vec![255, 255, 255, 255, 0, 0, 0, 255]);
        // Already small enough: untouched.
        assert_eq!(shrink(&image, 10), image);
    }

    #[test]
    fn peaks_are_normalised_to_the_loudest() {
        let mut samples = vec![0.0_f32; 960];
        for s in samples.iter_mut().skip(480).take(10) {
            *s = 0.1;
        }
        let peaks = normalised_peaks(&PcmBuffer::new(48_000, 1, samples));
        assert_eq!(peaks.len(), WAVE_BINS);
        assert_eq!(peaks.iter().max(), Some(&255));
        assert_eq!(peaks.first(), Some(&0));
    }

    #[test]
    fn captions_describe_every_kind() {
        let mut doc = Document::default();
        doc.add_symbol("box", SymbolKind::MovieClip).unwrap();
        let state = PreviewState::default();
        assert!(caption(&doc, "box", &state).starts_with("Movie clip, 1 frame"));
        assert_eq!(kind_of(&doc, "box"), Kind::Still);
        assert_eq!(kind_of(&doc, "nothing"), Kind::Other);
        let noted = PreviewState { note: "no audio data".into(), ..PreviewState::default() };
        assert!(caption(&doc, "box", &noted).ends_with("(no audio data)"));
    }
}
