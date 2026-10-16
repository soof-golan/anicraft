//! The window's side of the Library preview: asking the worker for pictures, running the play button and keeping the
//! sound in step. The pure parts (frames, captions, rendering) are in [`crate::preview`].

use std::time::Instant;

use anim_core::num;
use egui::{ColorImage, Context, TextureOptions};

use super::AniApp;
use crate::preview::{Kind, PreviewData, PreviewState, kind_of};
use crate::worker::Job;

impl AniApp {
    /// Keeps the preview of the selected library item current and moves a playing one along.
    ///
    /// Called every frame after the worker has been given the current document, so the request it queues is drawn
    /// from the document on screen.
    pub(super) fn drive_preview(&mut self, ctx: &Context) {
        let Some(name) = self.ui.library_selected.clone() else {
            self.stop_preview();
            self.ui.preview = PreviewState::default();
            return;
        };
        let doc = self.host.session().doc().document();
        if doc.library.find(&name).is_none() {
            self.ui.library_selected = None;
            self.stop_preview();
            self.ui.preview = PreviewState::default();
            return;
        }
        let kind = kind_of(doc, &name);
        let fps = doc.frame_rate;
        if self.ui.preview.name.as_deref() != Some(name.as_str()) {
            self.stop_preview();
            self.ui.preview = PreviewState { name: Some(name.clone()), ..PreviewState::default() };
        }
        let now = Instant::now();
        // The main timeline playing and the preview playing would talk over each other.
        if self.ui.playback.is_some() && self.ui.preview.started.is_some() {
            self.stop_preview();
        }
        if matches!(kind, Kind::Sound) && self.ui.preview.started.is_some() && self.ui.preview.sound_finished(now) {
            self.stop_preview();
        }
        let revision = self.sync.revision;
        let st = &mut self.ui.preview;
        let playing = st.started.is_some();
        let wanted = match kind {
            Kind::Clip { frames } if playing => st.clip_frame(frames, fps, now),
            _ => 0,
        };
        let symbol = matches!(kind, Kind::Clip { .. } | Kind::Still);
        let redraw = symbol && st.revision != revision;
        let needed = !matches!(kind, Kind::Other) && (st.requested_frame.is_none() || redraw || (symbol && st.requested_frame != Some(wanted)));
        if needed {
            st.requested_frame = Some(wanted);
            st.revision = revision;
            self.worker.send(Job::Preview { name, frame: wanted, playing: playing && symbol });
        }
        if playing {
            ctx.request_repaint();
        }
    }

    /// The worker finished a preview request.
    pub(super) fn preview_ready(&mut self, ctx: &Context, name: &str, data: PreviewData) {
        let st = &mut self.ui.preview;
        if st.name.as_deref() != Some(name) {
            return;
        }
        match data {
            PreviewData::Picture { image, content } => {
                let size = [num::u32_to_usize(image.width), num::u32_to_usize(image.height)];
                let colour = ColorImage::from_rgba_premultiplied(size, &image.premultiplied_rgba);
                st.texture_size = [image.width, image.height];
                st.content = content;
                st.note.clear();
                match &mut st.texture {
                    Some(tex) => tex.set(colour, TextureOptions::LINEAR),
                    None => st.texture = Some(ctx.load_texture(format!("preview:{name}"), colour, TextureOptions::LINEAR)),
                }
            }
            PreviewData::Sound { peaks, pcm } => {
                st.peaks = peaks;
                st.pcm = Some(pcm);
                st.note.clear();
                if std::mem::take(&mut st.autoplay) {
                    self.preview_play();
                }
            }
            PreviewData::Nothing(note) => note.clone_into(&mut st.note),
        }
    }

    /// Selects library item `name` and starts it playing, for `--library` screenshots. A sound starts once it is
    /// decoded; a clip right away.
    pub(super) fn show_library_item(&mut self, name: &str, play: bool) {
        self.ui.library_selected = Some(name.to_owned());
        self.ui.preview = PreviewState { name: Some(name.to_owned()), autoplay: play, ..PreviewState::default() };
        if play && matches!(self.selected_preview(), Some((_, Kind::Clip { .. }, _))) {
            self.ui.preview.autoplay = false;
            self.preview_play();
        }
    }

    /// True when the Library panel shows what the selected item looks like (or says why it cannot), so a screenshot
    /// does not catch the empty box that precedes the worker's answer.
    pub(super) fn preview_settled(&self) -> bool {
        let Some((_, kind, _)) = self.selected_preview() else { return true };
        let st = &self.ui.preview;
        matches!(kind, Kind::Other) || st.texture.is_some() || !st.peaks.is_empty() || !st.note.is_empty()
    }

    fn selected_preview(&self) -> Option<(String, Kind, f64)> {
        let name = self.ui.library_selected.clone()?;
        let doc = self.host.session().doc().document();
        Some((name.clone(), kind_of(doc, &name), doc.frame_rate))
    }

    /// The play button: start the clip or sound from the beginning (or where a seek left it).
    pub(super) fn preview_play(&mut self) {
        let Some((_, kind, _)) = self.selected_preview() else { return };
        self.start_preview(kind, 0);
    }

    /// A click or drag on the progress bar: play from that point.
    pub(super) fn preview_seek(&mut self, permille: u16) {
        let Some((_, kind, _)) = self.selected_preview() else { return };
        self.start_preview(kind, permille);
    }

    fn start_preview(&mut self, kind: Kind, permille: u16) {
        let fps = self.host.session().doc().document().frame_rate;
        if !matches!(kind, Kind::Clip { .. } | Kind::Sound) {
            return;
        }
        self.stop_playback();
        let now = Instant::now();
        let st = &mut self.ui.preview;
        if matches!(kind, Kind::Sound) {
            let Some(pcm) = st.pcm.clone() else { return };
            st.seek(kind, fps, permille, now);
            let offset = st.sound_seconds() * f64::from(permille) / 1000.0;
            self.audio.play(pcm, offset);
        } else {
            st.seek(kind, fps, permille, now);
        }
    }

    /// The stop button: back to the first frame and silence.
    pub(super) fn preview_stop(&mut self) {
        self.stop_preview();
    }

    fn stop_preview(&mut self) {
        if self.ui.preview.started.take().is_some() {
            self.audio.pause();
        }
    }
}
