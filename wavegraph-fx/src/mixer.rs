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
    /// Track level in units of 1/128 (128 is unity).
    Level {
        /// Zero-based track index; out-of-range indices are ignored.
        track: u8,
        /// Defaults to a limit of 128. With [boost](ChannelMixer::with_boost),
        /// the full `u8` range is available.
        level: u8,
    },
    /// Post-fader level for the first send, 0..=128.
    Send {
        /// Zero-based track index; out-of-range indices are ignored.
        track: u8,
        /// Post-fader send gain in units of 1/128; values above 128 are clamped.
        level: u8,
    },
    /// Open selected tracks on one send without overwriting their fader settings.
    SendGate {
        /// Zero-based send index.
        send: u8,
        /// Bit zero gates track zero. A set bit passes the stored send level.
        tracks: u8,
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
    gates: [u8; OUTPUTS],
    level_limit: u8,
    dry_shift: u8,
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
        Self {
            gains,
            gates: [u8::MAX; OUTPUTS],
            level_limit: 128,
            dry_shift: 0,
        }
    }

    /// Allow track faders up to 255 (about +6 dB). Effect sends remain capped at unity.
    #[must_use]
    pub const fn with_boost(mut self) -> Self {
        self.level_limit = u8::MAX;
        self
    }

    /// Reserve up to three bits of headroom on the dry outlet before Q23 conversion.
    /// Three bits accommodate four fully boosted tracks. Restore this gain at
    /// the master insert. Both sends retain their original levels.
    #[must_use]
    pub const fn with_dry_headroom(mut self, bits: u8) -> Self {
        self.dry_shift = if bits > 3 { 3 } else { bits };
        self
    }

    fn control(&mut self, message: MixerMessage) {
        let (outlet, track, level) = match message {
            MixerMessage::SendGate { send, tracks } => {
                if let Some(gate) = self.gates.get_mut(usize::from(send).saturating_add(1)) {
                    *gate = tracks;
                }
                return;
            }
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
            *gain = level.min(if outlet == 0 { self.level_limit } else { 128 });
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
    reason = "Q23 times 255 fits i32; post-fader sends are at most unity and four tracks fit i32"
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
        let mut start = 0;
        for event in cx.events {
            self.render(cx.inputs, cx.outputs, start..event.frame);
            self.control(event.message);
            start = event.frame;
        }
        self.render(cx.inputs, cx.outputs, start..cx.frames);
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Q23 samples, byte gains and sums of up to four tracks fit i32"
)]
impl<const TRACKS: usize, const OUTPUTS: usize> ChannelMixer<TRACKS, OUTPUTS> {
    fn render(
        &self,
        inputs: &[&[I24]],
        outputs: &mut [&mut [I24]],
        frames: core::ops::Range<usize>,
    ) {
        if frames.is_empty() {
            return;
        }
        let mut gains = self.gains.map(|row| row.map(i32::from));
        for (send, &gate) in gains[1..].iter_mut().zip(&self.gates[1..]) {
            if gate != u8::MAX {
                for (track, level) in send.iter_mut().enumerate() {
                    if gate & (1 << track) == 0 {
                        *level = 0;
                    }
                }
            }
        }
        if let (Some(inputs), Some(outputs)) = (
            inputs.first_chunk::<TRACKS>(),
            outputs.first_chunk_mut::<OUTPUTS>(),
        ) {
            let signals = inputs.each_ref().map(|signal| &signal[frames.clone()]);
            let mut buses = outputs.each_mut().map(|out| &mut out[frames.clone()]);
            for frame in 0..frames.len() {
                let post: [i32; TRACKS] = core::array::from_fn(|track| {
                    (signals[track][frame].inner() * gains[0][track]) >> 7
                });
                buses[0][frame] = pcm(post.iter().sum::<i32>() >> self.dry_shift);
                for (bus, send) in buses[1..].iter_mut().zip(&gains[1..]) {
                    let sum = post
                        .iter()
                        .zip(send)
                        .map(|(sample, gain)| (sample * gain) >> 7)
                        .sum();
                    bus[frame] = pcm(sum);
                }
            }
            return;
        }
        for frame in frames {
            let mut sums = [0_i32; OUTPUTS];
            for (track, input) in inputs.iter().enumerate().take(TRACKS) {
                let post_fader = (input[frame].inner() * gains[0][track]) >> 7;
                sums[0] += post_fader;
                for (sum, gains) in sums[1..].iter_mut().zip(&gains[1..]) {
                    *sum += (post_fader * gains[track]) >> 7;
                }
            }
            sums[0] >>= self.dry_shift;
            for (out, sum) in outputs.iter_mut().zip(sums) {
                out[frame] = pcm(sum);
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
    fn boosted_fader_keeps_headroom_before_post_fader_summing() {
        let mut mixer = ChannelMixer::<2, 3>::new().with_boost();
        let inputs = [
            [I24::new_unchecked(-(1 << 23))],
            [I24::new_unchecked(1 << 22)],
        ];
        let events = [
            event(
                0,
                MixerMessage::Level {
                    track: 0,
                    level: 255,
                },
            ),
            event(
                0,
                MixerMessage::Level {
                    track: 1,
                    level: 255,
                },
            ),
            event(
                0,
                MixerMessage::Send {
                    track: 0,
                    level: 255,
                },
            ),
            event(
                0,
                MixerMessage::SendTo {
                    send: 1,
                    track: 1,
                    level: 64,
                },
            ),
        ];
        let mut dry = [I24::default()];
        let mut reverb = dry;
        let mut delay = dry;
        mixer.process(&mut ProcessContext {
            frames: 1,
            inputs: &[&inputs[0], &inputs[1]],
            outputs: &mut [&mut dry, &mut reverb, &mut delay],
            events: &events,
            emit: &mut Err,
        });
        // A track can exceed Q23 internally. Only the summed outputs saturate.
        assert_eq!(dry[0].inner(), -(255 << 15));
        assert_eq!(reverb[0].inner(), -(1 << 23));
        assert_eq!(delay[0].inner(), 255 << 14);
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
