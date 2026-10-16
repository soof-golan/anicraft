//! Sound during playback: a lazily started `PlaybackActor` that plays the soundtrack the render worker mixed.
//!
//! The playhead is driven by the wall clock, not by the audio device, so a missing or failing audio device never
//! stops the animation; the sound simply starts at the offset that matches the playhead when it is ready.

use anim_audio::{ClockSource, PcmBuffer, PlaybackActor, PlaybackCommand, PlaybackEvent};
use anim_core::Tick;

/// The playback side of the editor.
#[derive(Debug, Default)]
pub struct Audio {
    actor: Option<PlaybackActor>,
    /// A device was tried and failed (headless machines): do not try again every play.
    failed: bool,
    /// Where the clock comes from, once the actor reported it.
    pub clock_source: Option<ClockSource>,
    /// A message for the status bar (device errors).
    pub message: Option<String>,
}

impl Audio {
    fn ensure(&mut self) -> Option<&PlaybackActor> {
        if self.actor.is_none() && !self.failed {
            match PlaybackActor::spawn() {
                Ok(actor) => self.actor = Some(actor),
                Err(e) => {
                    self.failed = true;
                    self.message = Some(format!("no audio output: {e}"));
                }
            }
        }
        self.actor.as_ref()
    }

    /// Plays `track` from `offset` seconds into it.
    pub fn play(&mut self, track: PcmBuffer, offset_seconds: f64) {
        if let Some(actor) = self.ensure() {
            // A stopped actor only means there is no sound; the animation keeps playing.
            let _ = actor.send(PlaybackCommand::Load(track));
            let _ = actor.send(PlaybackCommand::Play { from: Tick::from_seconds_f64(offset_seconds.max(0.0)) });
        }
    }

    /// Silences playback.
    pub fn pause(&mut self) {
        if let Some(actor) = &self.actor {
            let _ = actor.send(PlaybackCommand::Pause);
        }
    }

    /// Drains the actor's events so its queue does not grow.
    pub fn poll(&mut self) {
        let Some(actor) = &self.actor else { return };
        let (_, events) = actor.poll();
        for event in events {
            match event {
                PlaybackEvent::Ready { clock_source, .. } => self.clock_source = Some(clock_source),
                PlaybackEvent::Error(message) => self.message = Some(message),
                _ => {}
            }
        }
    }
}
