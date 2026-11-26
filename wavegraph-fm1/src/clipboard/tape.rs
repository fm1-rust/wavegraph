//! One transport-synced tape head over the shared sampler clipboard.

use super::{Action, Options, PCM, Recorder, Request, State};
use crate::{
    assets::CLIPBOARD_SLOT,
    instrument::{DRY_HEADROOM, Part, Track},
    mixer::Mixer,
    panel::CHANNELS,
    sequencer::Transport,
};
use core::sync::atomic::Ordering;
use wavegraph_sample::{I24, types::i24};

// The transport ranges from 30 to 300 BPM. Q4.28 holds the largest (10x)
// tape speed, with less than 0.003 PCM frame of rounding over the longest loop.
const FRACTION_BITS: u32 = 28;
const FRACTION_MASK: u32 = (1 << FRACTION_BITS) - 1;

/// The mixer reads the old pass before the recorder prints the new strip audio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Playback {
    // Separate the whole and fractional sample so the audio loop uses native
    // 32-bit adds and graph events keep their four-byte alignment on the FM-1.
    fraction: u32,
    step: u32,
    cell: u16,
    frames: u16,
    edge: usize,
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "The clip holds at most 32,768 PCM16 frames and the caller supplies one audio block."
)]
impl Playback {
    fn at(&mut self, frame: usize) -> Option<(u16, u32)> {
        if self.edge == frame {
            self.cell = 0;
            self.fraction = 0;
        }
        if self.cell == u16::MAX {
            return None;
        }
        let position = (self.cell, self.fraction);
        let advanced = self.fraction + self.step;
        let next = u32::from(self.cell) + (advanced >> FRACTION_BITS);
        if next < u32::from(self.frames) {
            self.cell = u16::try_from(next).expect("within the clip");
            self.fraction = advanced & FRACTION_MASK;
        } else {
            self.cell = self.frames - 1;
            self.fraction = FRACTION_MASK;
        }
        Some(position)
    }

    fn end(self, count: usize) -> Option<u64> {
        let (position, count) = if self.edge < count {
            (0, count - self.edge)
        } else if self.cell != u16::MAX {
            (
                (u64::from(self.cell) << FRACTION_BITS) | u64::from(self.fraction),
                count,
            )
        } else {
            return None;
        };
        let end = (u64::from(self.frames) << FRACTION_BITS) - 1;
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        Some((position + u64::from(self.step) * u64::from(count)).min(end))
    }

    pub(crate) fn mix(mut self, dry: &mut [I24]) {
        for (frame, out) in dry.iter_mut().enumerate() {
            let Some((cell, fraction)) = self.at(frame) else {
                continue;
            };
            let i = usize::from(cell);
            let next = if i + 1 == usize::from(self.frames) {
                0
            } else {
                i + 1
            };
            let a = i32::from(PCM[i].load(Ordering::Relaxed));
            let b = i32::from(PCM[next].load(Ordering::Relaxed));
            let fraction = (fraction >> (FRACTION_BITS - 15)).cast_signed();
            let sample = (a << 8) + (((b - a) * fraction) >> 7);
            let sum = out.inner() + (sample >> DRY_HEADROOM);
            *out = I24::new_unchecked(sum.clamp(i24::MIN.inner(), i24::MAX.inner()));
        }
    }
}

/// Average all input frames landing on one tape cell. A faster tape head may
/// cross several cells in one input frame, in which case they share that sample.
#[derive(Default)]
struct Layer {
    cell: Option<u16>,
    sum: i64,
    count: u32,
}

impl Layer {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A pending cell always has at least one PCM16 input."
    )]
    fn flush(&mut self, end: u16) {
        if let Some(cell) = self.cell.take() {
            let mean = if self.count == 1 {
                self.sum
            } else {
                self.sum / i64::from(self.count)
            };
            let value = i16::try_from(mean).expect("PCM16 mean");
            for sample in &PCM[usize::from(cell)..usize::from(end.max(cell + 1))] {
                sample.store(
                    sample.load(Ordering::Relaxed).saturating_add(value),
                    Ordering::Relaxed,
                );
            }
            self.sum = 0;
            self.count = 0;
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "At most one four-bar cycle of PCM16 contributes to a single cell."
    )]
    fn add(&mut self, cell: u16, sample: i32) {
        if self.cell != Some(cell) {
            self.flush(cell);
            self.cell = Some(cell);
        }
        self.sum += i64::from(sample);
        self.count += 1;
    }
}

#[derive(Default)]
pub(super) struct Tape {
    pub(super) selected: bool,
    position: Option<u64>,
    transport: Option<Transport>,
    speed: Option<(u16, u16, u8)>,
    increment: u32,
    layer: Layer,
}

impl Tape {
    fn punch_out(&mut self) {
        let next = self.position.map_or(0, |p| {
            u16::try_from(p >> FRACTION_BITS).expect("PCM slot index")
        });
        self.layer.flush(next);
    }

    pub(super) fn stop(&mut self) {
        self.punch_out();
        self.position = None;
    }
}

impl Recorder {
    pub fn loop_beats(&self, options: Options) -> u8 {
        self.status.take.map_or_else(
            || {
                if self.status.state == State::Recording {
                    self.beats
                } else {
                    options.beats()
                }
            },
            |take| take.beats,
        )
    }

    /// Plan monitoring before the graph renders. The source and destination of
    /// an overdub never include the tape return or the master effects.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Valid audio clock and positive take length; rate products fit u64."
    )]
    pub fn prepare_tape(
        &mut self,
        request: Request,
        tracks: &[Track; CHANNELS],
        mixer: Mixer,
        transport: Transport,
        edge: Option<usize>,
    ) -> Option<Playback> {
        let old = self.tape.transport.replace(transport);
        if !self.tape.selected || old.is_some_and(|t| t.playing() && !t.same_run(transport)) {
            self.interrupt();
            if self.status.state == State::Armed {
                self.status.state = State::Idle;
            }
        }
        self.tape.selected = true;
        if self.status.state == State::Recording && old.is_some_and(|t| t.bpm != transport.bpm) {
            self.interrupt();
        }
        if self.serial != request.serial {
            self.serial = request.serial;
            match request.action {
                Action::Clear => self.clear_stopped(transport),
                Action::Sample => (),
                Action::Mix | Action::Strip => match self.status.state {
                    State::Armed | State::Recording => self.status.state = State::Idle,
                    State::Dubbing => {
                        self.tape.punch_out();
                        self.status.state = State::Idle;
                    }
                    _ => {
                        self.status.source = request.source;
                        self.status.mix = request.action == Action::Mix;
                        self.status.state = State::Armed;
                    }
                },
            }
        }
        if matches!(
            self.status.state,
            State::Armed | State::Recording | State::Dubbing
        ) {
            let source = &tracks[self.status.source.index()];
            let clipboard = |t: &Track| {
                t.assignment.part == Part::Sampler && t.voice.waveform == CLIPBOARD_SLOT
            };
            let self_input = if self.status.mix {
                tracks
                    .iter()
                    .zip(mixer.0)
                    .any(|(t, strip)| strip.output_level() != 0 && clipboard(t))
            } else {
                clipboard(source)
            };
            if !self.status.mix && source.assignment != request.assignment {
                self.tape.punch_out();
                self.status.state = State::Interrupted;
            } else if self_input {
                self.tape.punch_out();
                self.status.state = State::SelfInput;
            }
        }
        if !transport.playing() {
            return None;
        }
        let (frames, beats) = if let Some(take) = self.status.take {
            (take.frames, take.beats)
        } else if self.status.state == State::Recording {
            // The first samples are already on tape. Monitor them on the exact
            // closing downbeat even if it falls in the middle of this block.
            let edge = u32::try_from(edge?).expect("audio block");
            let tail = (u32::from(self.pending) + edge.min(self.remaining))
                .div_ceil(u32::from(self.divisor));
            (
                self.written + u16::try_from(tail).expect("bounded by tape length"),
                self.beats,
            )
        } else {
            return None;
        };
        if frames == 0 || (self.tape.position.is_none() && edge.is_none()) {
            return None;
        }
        let speed = (transport.bpm, frames, beats);
        if self.tape.speed != Some(speed) {
            let period = self.loop_frames(transport.bpm, beats);
            let step = ((u64::from(frames) << FRACTION_BITS) + period / 2) / period;
            self.tape.increment = u32::try_from(step).expect("at most 10x tape speed");
            self.tape.speed = Some(speed);
        }
        let position = self
            .tape
            .position
            .unwrap_or(u64::from(u16::MAX) << FRACTION_BITS);
        Some(Playback {
            cell: u16::try_from(position >> FRACTION_BITS).expect("PCM slot or stopped"),
            fraction: u32::try_from(position & u64::from(FRACTION_MASK)).expect("Q28 fraction"),
            step: self.tape.increment,
            frames,
            edge: edge.unwrap_or(usize::MAX),
        })
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A PCM16 times Q7 gain fits i32."
    )]
    pub fn print_tape(
        &mut self,
        request: Request,
        transport: Transport,
        edge: Option<usize>,
        mut head: Option<Playback>,
        input: impl ExactSizeIterator<Item = I24>,
    ) {
        if !transport.playing() {
            return;
        }
        self.tape.position = head.and_then(|h| h.end(input.len()));
        self.status.monitoring = self.tape.position.is_some();
        if !matches!(
            self.status.state,
            State::Armed | State::Recording | State::Dubbing
        ) || (self.status.state == State::Armed && edge.is_none())
        {
            return;
        }
        for (frame, sample) in input.enumerate() {
            if edge == Some(frame) {
                match self.status.state {
                    State::Recording => self.finish(),
                    State::Dubbing => self.tape.layer.flush(head.expect("playing loop").frames),
                    State::Armed if self.status.take.is_some() => {
                        self.status.state = State::Dubbing
                    }
                    State::Armed => self.begin(transport.bpm, request.options),
                    _ => (),
                }
            }
            let position = head.as_mut().and_then(|h| h.at(frame));
            let incoming = ((sample.inner() >> 8) * i32::from(request.options.level)) >> 7;
            match self.status.state {
                State::Recording => {
                    self.record_sample(incoming);
                }
                State::Dubbing => {
                    let (cell, _) = position.expect("dubbing starts at the loop edge");
                    self.tape.layer.add(cell, incoming);
                }
                _ => (),
            }
        }
    }
}
