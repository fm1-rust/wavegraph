#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod chord;
mod clock;

pub use chord::Chord;
pub use clock::Clock;
use wavegraph_core::{Event, Port, Process, ProcessContext, midi::MidiMessage};

/// Pitch traversal within an arpeggio.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    /// Lowest to highest.
    #[default]
    Up,
    /// Highest to lowest.
    Down,
    /// Ascend and descend, playing each endpoint once.
    UpDown,
    /// Follow the order in which the keys were struck.
    AsPlayed,
    /// Pick a pseudo-random member of the chord at each step.
    Random,
}

/// Musical duration of one step, relative to a quarter-note beat.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Division {
    /// One step per beat.
    Quarter = 1,
    /// Two steps per beat.
    Eighth = 2,
    /// Three steps per beat.
    EighthTriplet = 3,
    /// Four steps per beat.
    #[default]
    Sixteenth = 4,
    /// Six steps per beat.
    SixteenthTriplet = 6,
    /// Eight steps per beat.
    ThirtySecond = 8,
}

/// Arpeggiator settings. Range and gate values are bounded at playback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Note order.
    pub mode: Mode,
    /// Step length.
    pub division: Division,
    /// Number of octave passes, from 1 to 4. Pitches above MIDI 127 are skipped.
    pub octaves: u8,
    /// Sounding fraction of each step, from 1 to 100 percent.
    pub gate: u8,
    /// Sustain the chord after key release until the next chord is played.
    pub latch: bool,
}

impl Config {
    /// Upward sixteenths over one octave with a 60% gate and momentary keys.
    pub const INITIAL: Self = Self {
        mode: Mode::Up,
        division: Division::Sixteenth,
        octaves: 1,
        gate: 60,
        latch: false,
    };
}

impl Default for Config {
    fn default() -> Self {
        Self::INITIAL
    }
}

/// A Pd-style object with one MIDI message inlet and one MIDI message outlet.
/// Connect one instance per routed channel.
#[derive(Clone, Debug)]
pub struct Arpeggiator {
    chord: Chord,
    clock: Clock,
    config: Config,
    bpm: u16,
}

impl Arpeggiator {
    /// Construct for an audio clock of `numerator / denominator` frames per second.
    /// Accepts sample rates from 1 kHz through 768 kHz. Starts at 120 BPM.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        Some(Self {
            chord: Chord::new(),
            clock: Clock::new(numerator, denominator)?,
            config: Config::INITIAL,
            bpm: 120,
        })
    }

    /// Apply new settings. Tempo is in quarter notes per minute, from 30 to 300.
    pub fn configure(&mut self, config: Config, bpm: u16) {
        if self.config.latch && !config.latch {
            self.chord.release_latch();
        }
        self.config = config;
        self.bpm = bpm;
    }

    fn span(
        &mut self,
        start: usize,
        frames: usize,
        emit: &mut dyn FnMut(Event<MidiMessage>) -> Result<(), Event<MidiMessage>>,
    ) {
        let _ = self
            .clock
            .render(&self.chord, self.config, self.bpm, frames, |mut event| {
                event.frame = event.frame.saturating_add(start);
                emit(event)
            });
    }
}

impl<S> Process<MidiMessage, S> for Arpeggiator {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Message]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, MidiMessage, S>) {
        let mut start = 0;
        for event in cx.events {
            if event.frame > start {
                self.span(start, event.frame.saturating_sub(start), cx.emit);
                start = event.frame;
            }
            self.chord.input(event.message, self.config.latch);
            if let MidiMessage::Control(..) = event.message {
                if matches!(event.message, MidiMessage::Control(120 | 123, _)) {
                    let _ = self.clock.stop(|note| {
                        (cx.emit)(Event {
                            frame: start,
                            port: 0,
                            message: MidiMessage::Off(note),
                        })
                    });
                }
                let _ = (cx.emit)(Event { port: 0, ..*event });
            }
        }
        self.span(start, cx.frames.saturating_sub(start), cx.emit);
    }
}
