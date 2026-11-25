//! One channel records into a single session-wide PCM16 slot, owned by the DSP worker.
use crate::{
    assets::CLIPBOARD_SLOT,
    instrument::{Part, Track},
    panel::{Assignment, CHANNELS, Channel},
    sequencer::Transport,
};
use core::sync::atomic::{AtomicI16, Ordering};
use wavegraph_sample::I24;
use wavegraph_sampler::Clip;

const FRAMES: usize = 32_768;
// SAFETY: the application linker reserves this 64 KiB independently of stacks,
// bootloader plug-ins and the BSP's .bss. Startup initializes it before CPU1 starts.
#[cfg_attr(target_os = "none", unsafe(link_section = ".wavegraph.clipboard"))]
pub(super) static PCM: [AtomicI16; FRAMES] = [const { AtomicI16::new(0) }; FRAMES];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Options {
    length: u8,
    pub root: u8,
}

impl Options {
    pub const INITIAL: Self = Self {
        length: 1,
        root: 60,
    };
    const BEATS: [u8; 4] = [1, 4, 8, 16];

    pub fn turn(&mut self, length: i8, root: i8) {
        self.length = self.length.saturating_add_signed(length).min(3);
        self.root = self.root.saturating_add_signed(root).min(127);
    }

    pub fn beats(self) -> u8 {
        Self::BEATS[usize::from(self.length)]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Request {
    serial: u32,
    source: Channel,
    assignment: Assignment,
    /// The recorder latches the current length and root at the first recorded frame.
    pub options: Options,
}

impl Request {
    pub const INITIAL: Self = Self {
        serial: 0,
        source: Channel::One,
        assignment: Assignment::new(Part::Bass),
        options: Options::INITIAL,
    };

    pub fn press(&mut self, source: Channel, assignment: Assignment) {
        self.serial = self.serial.wrapping_add(1);
        self.source = source;
        self.assignment = assignment;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum State {
    #[default]
    Idle,
    Armed,
    Recording,
    Interrupted,
    SelfInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Take {
    serial: u32,
    pub frames: u16,
    pub hz: u16,
    pub root: u8,
}

impl Take {
    pub fn clip(self) -> Clip<'static> {
        Clip::from_shared_pcm16(&PCM[..usize::from(self.frames)], self.hz.into(), self.root)
            .expect("recording has a positive clock and MIDI root")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Status {
    pub state: State,
    pub source: Channel,
    pub take: Option<Take>,
}

pub(super) struct Recorder {
    pub status: Status,
    serial: u32,
    root: u8,
    rate: (u32, u32),
    remaining: u32,
    divisor: u16,
    hz: u16,
    sum: i32,
    pending: u16,
    written: u16,
}

impl Recorder {
    pub fn new(rate: (u32, u32)) -> Self {
        Self {
            status: Status::default(),
            serial: 0,
            root: Options::INITIAL.root,
            rate,
            remaining: 0,
            divisor: 1,
            hz: 0,
            sum: 0,
            pending: 0,
            written: 0,
        }
    }

    pub fn interrupt(&mut self) {
        if self.status.state == State::Recording {
            self.status.state = State::Interrupted;
        }
    }

    fn press(&mut self, request: Request) {
        self.serial = request.serial;
        match self.status.state {
            State::Armed => self.status.state = State::Idle,
            State::Recording => self.finish(),
            _ => {
                self.status.state = State::Armed;
                self.status.source = request.source;
            }
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Nonzero validated audio clock, tempo and capacity; 16 beats fit the 64-bit numerator."
    )]
    fn begin(&mut self, bpm: u16, options: Options) {
        self.root = options.root;
        let frames = (u64::from(self.rate.0) * 60 * u64::from(options.beats()))
            .div_ceil(u64::from(self.rate.1) * u64::from(bpm));
        self.remaining = u32::try_from(frames).expect("FM-1 capture length");
        self.divisor = u16::try_from(frames.div_ceil(FRAMES as u64)).expect("bounded decimation");
        let denominator = u64::from(self.rate.1) * u64::from(self.divisor);
        self.hz = u16::try_from((u64::from(self.rate.0) + denominator / 2) / denominator)
            .expect("FM-1 mono PCM rate");
        self.status.take = None;
        self.status.state = State::Recording;
        self.written = 0;
        self.sum = 0;
        self.pending = 0;
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A nonempty group contains at most 44 PCM16 samples. The recording is bounded by the slot."
    )]
    fn store(&mut self) {
        let value = i16::try_from(self.sum / i32::from(self.pending)).expect("PCM16 average");
        PCM[usize::from(self.written)].store(value, Ordering::Relaxed);
        self.written += 1;
        self.sum = 0;
        self.pending = 0;
    }

    fn finish(&mut self) {
        if self.pending > 0 {
            self.store();
        }
        self.status.take = Some(Take {
            serial: self.serial,
            frames: self.written,
            hz: self.hz,
            root: self.root,
        });
        self.status.state = State::Idle;
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The recorder stops at its positive target, and divides each bounded sum before the next group."
    )]
    pub fn process<const N: usize>(
        &mut self,
        request: Request,
        tracks: &[Track; CHANNELS],
        transport: Transport,
        downbeat: Option<usize>,
        buses: &[[I24; N]],
    ) {
        if self.serial != request.serial {
            self.press(request);
        }
        if !matches!(self.status.state, State::Armed | State::Recording) {
            return;
        }
        let source = request.source;
        let track = &tracks[source.index()];
        if track.assignment != request.assignment {
            self.status.state = State::Interrupted;
            return;
        }
        if track.assignment.part == Part::Sampler && track.voice.waveform == CLIPBOARD_SLOT {
            self.status.state = State::SelfInput;
            return;
        }
        let mut input = buses[source.source_bus()].as_slice();
        if self.status.state == State::Armed {
            let start = if transport.playing() {
                downbeat
            } else {
                input.iter().position(|s| s.inner().unsigned_abs() >= 256)
            };
            let Some(start) = start else { return };
            self.begin(transport.bpm, request.options);
            input = &input[start..];
        }
        for sample in input {
            self.sum += sample.inner() >> 8;
            self.pending += 1;
            self.remaining -= 1;
            if self.pending == self.divisor {
                self.store();
            }
            if self.remaining == 0 {
                self.finish();
                break;
            }
        }
    }
}
