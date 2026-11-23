//! Allocation-free per-track level controls and post-fader global-effect sends.
//!
//! Outlet zero carries the dry sum. Each further outlet carries an independent
//! post-fader send. Control messages take effect at their sample offsets.

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
    /// Post-fader level for the first send, 0..=128.
    Send {
        /// Zero-based track index; out-of-range indices are ignored.
        track: u8,
        /// Post-fader send gain in units of 1/128; values above 128 are clamped.
        level: u8,
    },
    /// Set the level on a specific post-fader send.
    SendTo {
        /// Zero-based send index. Send 0 is outlet 1. Unavailable sends are ignored.
        send: u8,
        /// Zero-based track index. Unavailable tracks are ignored.
        track: u8,
        /// Gain in units of 1/128. Values above 128 are clamped.
        level: u8,
    },
}

/// Fixed-size mixer with one dry output and `OUTPUTS - 1` post-fader sends.
/// The default layout has one send.
///
/// Keep the instruments upstream of this node so their persistent DSP state
/// is independent of mixer page changes and graph replacements.
#[derive(Clone, Copy, Debug)]
pub struct ChannelMixer<const TRACKS: usize, const OUTPUTS: usize = 2> {
    gains: [[u8; TRACKS]; OUTPUTS],
}

impl<const TRACKS: usize, const OUTPUTS: usize> ChannelMixer<TRACKS, OUTPUTS> {
    /// Start with unity track levels and zero effect sends.
    ///
    /// # Panics
    /// A compile-time assertion requires 1–4 tracks and at least two outputs.
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(TRACKS > 0 && TRACKS <= 4, "mixer supports 1..=4 tracks") };
        const { assert!(OUTPUTS >= 2, "mixer needs a dry output and a send") };
        let mut gains = [[0; TRACKS]; OUTPUTS];
        gains[0] = [128; TRACKS];
        Self { gains }
    }

    fn control(&mut self, message: MixerMessage) {
        let (outlet, track, level) = match message {
            MixerMessage::Level { track, level } => (0, track, level),
            MixerMessage::Send { track, level } => (1, track, level),
            MixerMessage::SendTo { send, track, level } => {
                (usize::from(send).saturating_add(1), track, level)
            }
        };
        if let Some(gain) = self
            .gains
            .get_mut(outlet)
            .and_then(|g| g.get_mut(usize::from(track)))
        {
            *gain = level.min(128);
        }
    }
}

impl<const TRACKS: usize, const OUTPUTS: usize> Default for ChannelMixer<TRACKS, OUTPUTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Each Q23 sample times a gain <=128 fits i32; sum is bounded for four tracks"
)]
impl<const TRACKS: usize, const OUTPUTS: usize> Process<MixerMessage, I24>
    for ChannelMixer<TRACKS, OUTPUTS>
{
    fn inlets(&self) -> &[Port] {
        &[Port::Signal; TRACKS]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal; OUTPUTS]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, MixerMessage, I24>) {
        let mut events = cx.events.iter().peekable();
        for frame in 0..cx.frames {
            while events.peek().is_some_and(|event| event.frame == frame) {
                self.control(events.next().expect("peeked mixer event").message);
            }
            let mut sums = [0_i32; OUTPUTS];
            for (track, input) in cx.inputs.iter().enumerate().take(TRACKS) {
                let post_fader = (input[frame].inner() * i32::from(self.gains[0][track])) >> 7;
                sums[0] += post_fader;
                for (sum, gains) in sums[1..].iter_mut().zip(&self.gains[1..]) {
                    *sum += (post_fader * i32::from(gains[track])) >> 7;
                }
            }
            for (out, sum) in cx.outputs.iter_mut().zip(sums) {
                out[frame] = pcm(sum);
            }
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

    #[test]
    fn four_tracks_can_feed_both_sends_at_different_levels() {
        let mut mixer = ChannelMixer::<4, 3>::new();
        let inputs = [1024, -2048, 4096, -8192].map(|n| [I24::new_unchecked(n); 4]);
        let mut dry = [I24::default(); 4];
        let mut reverb = dry;
        let mut delay = dry;
        for track in 0..4 {
            let events = [
                event(0, MixerMessage::Level { track, level: 64 }),
                event(0, MixerMessage::Send { track, level: 32 }),
                event(
                    0,
                    MixerMessage::SendTo {
                        send: 1,
                        track,
                        level: 128,
                    },
                ),
                event(
                    1,
                    MixerMessage::SendTo {
                        send: 1,
                        track,
                        level: 64,
                    },
                ),
                event(2, MixerMessage::Level { track, level: 0 }),
            ];
            let i = usize::from(track);
            mixer.process(&mut ProcessContext {
                frames: 4,
                inputs: &inputs.each_ref().map(|s| &s[..]),
                outputs: &mut [&mut dry, &mut reverb, &mut delay],
                events: &events,
                emit: &mut |e| Err(e),
            });
            let post = inputs[i][0].inner() >> 1;
            assert_eq!(delay.map(I24::inner), [post, post >> 1, 0, 0]);
            assert_eq!(reverb.map(I24::inner), [post >> 2, post >> 2, 0, 0]);
            assert_eq!(dry[0].inner() - dry[2].inner(), post);
        }
    }
}
