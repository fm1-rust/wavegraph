//! Allocation-free per-track level controls and post-fader global-effect sends.
//!
//! The mixer emits two buses: the dry sum and one shared effect send. Add
//! another output when a second global effect is introduced. Control messages
//! are sample-offset-aware; all arithmetic uses Q23 with bounded i32 products.

use super::pcm;
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::I24;

/// Changes applied at the start of the specified audio frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixerMessage {
    /// Track level, 0..=128 (128 is unity).
    Level {
        /// Zero-based track index; out-of-range indices are ignored.
        track: u8,
        /// Track gain in units of 1/128; values above 128 are clamped.
        level: u8,
    },
    /// Post-fader send to the shared effect, 0..=128.
    Send {
        /// Zero-based track index; out-of-range indices are ignored.
        track: u8,
        /// Post-fader send gain in units of 1/128; values above 128 are clamped.
        level: u8,
    },
}

/// Fixed-size mixer with one dry and one post-fader send output.
///
/// Keep the instruments upstream of this node so their persistent DSP state
/// is independent of mixer page changes and graph replacements.
#[derive(Clone, Copy, Debug)]
pub struct ChannelMixer<const TRACKS: usize> {
    levels: [u8; TRACKS],
    sends: [u8; TRACKS],
}

impl<const TRACKS: usize> ChannelMixer<TRACKS> {
    /// Start with unity track levels and zero effect sends.
    ///
    /// # Panics
    /// A compile-time assertion rejects sizes outside 1..=4.
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(TRACKS > 0 && TRACKS <= 4, "mixer supports 1..=4 tracks") };
        Self {
            levels: [128; TRACKS],
            sends: [0; TRACKS],
        }
    }

    fn control(&mut self, message: MixerMessage) {
        match message {
            MixerMessage::Level { track, level } => {
                if let Some(gain) = self.levels.get_mut(usize::from(track)) {
                    *gain = level.min(128);
                }
            }
            MixerMessage::Send { track, level } => {
                if let Some(gain) = self.sends.get_mut(usize::from(track)) {
                    *gain = level.min(128);
                }
            }
        }
    }
}

impl<const TRACKS: usize> Default for ChannelMixer<TRACKS> {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Each Q23 sample times a gain <=128 fits i32; sum is bounded for four tracks"
)]
impl<const TRACKS: usize> Process<MixerMessage, I24> for ChannelMixer<TRACKS> {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal; TRACKS]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal; 2]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, MixerMessage, I24>) {
        let mut events = cx.events.iter().peekable();
        for frame in 0..cx.frames {
            while events.peek().is_some_and(|event| event.frame == frame) {
                self.control(events.next().expect("peeked mixer event").message);
            }
            let mut dry = 0_i32;
            let mut send = 0_i32;
            for (track, input) in cx.inputs.iter().enumerate().take(TRACKS) {
                let post_fader = (input[frame].inner() * i32::from(self.levels[track])) >> 7;
                dry += post_fader;
                send += (post_fader * i32::from(self.sends[track])) >> 7;
            }
            cx.outputs[0][frame] = pcm(dry);
            cx.outputs[1][frame] = pcm(send);
        }
        // Match other WaveGraph processors: an empty block still updates state.
        if cx.frames == 0 {
            for event in cx.events {
                self.control(event.message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph_core::Event;

    fn event(frame: usize, message: MixerMessage) -> Event<MixerMessage> {
        Event {
            frame,
            port: 0,
            message,
        }
    }

    #[test]
    fn levels_and_post_fader_sends_are_independent() {
        let mut mixer = ChannelMixer::<2>::new();
        let one = [I24::new_unchecked(1024); 4];
        let two = [I24::new_unchecked(2048); 4];
        let mut dry = [I24::default(); 4];
        let mut send = [I24::default(); 4];
        let events = [
            event(
                0,
                MixerMessage::Level {
                    track: 0,
                    level: 64,
                },
            ),
            event(
                0,
                MixerMessage::Send {
                    track: 0,
                    level: 128,
                },
            ),
            event(
                0,
                MixerMessage::Send {
                    track: 1,
                    level: 64,
                },
            ),
            event(2, MixerMessage::Level { track: 0, level: 0 }),
        ];
        mixer.process(&mut ProcessContext {
            frames: 4,
            inputs: &[&one, &two],
            outputs: &mut [&mut dry, &mut send],
            events: &events,
            emit: &mut |event| Err(event),
        });
        assert_eq!(dry.map(I24::inner), [2560, 2560, 2048, 2048]);
        assert_eq!(send.map(I24::inner), [1536, 1536, 1024, 1024]);
    }

    #[test]
    fn four_tracks_and_gain_limits() {
        let mut mixer = ChannelMixer::<4>::new();
        let signal = [I24::new_unchecked(1 << 22); 1];
        let mut dry = [I24::default(); 1];
        let mut send = [I24::default(); 1];
        let events = [
            event(0, MixerMessage::Level { track: 0, level: 0 }),
            event(0, MixerMessage::Level { track: 1, level: 0 }),
            event(0, MixerMessage::Level { track: 2, level: 0 }),
            event(
                0,
                MixerMessage::Level {
                    track: 3,
                    level: 255,
                },
            ),
            event(
                0,
                MixerMessage::Send {
                    track: 3,
                    level: 255,
                },
            ),
        ];
        mixer.process(&mut ProcessContext {
            frames: 1,
            inputs: &[&signal, &signal, &signal, &signal],
            outputs: &mut [&mut dry, &mut send],
            events: &events,
            emit: &mut |event| Err(event),
        });
        assert_eq!(dry[0].inner(), 1 << 22);
        assert_eq!(send[0].inner(), 1 << 22);
    }
}
