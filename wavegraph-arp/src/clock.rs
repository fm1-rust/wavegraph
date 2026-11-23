//! Fractional sample timing shared by the patch object and the FM-1 audio task.

use crate::{Chord, Config, Division, Mode};
use wavegraph_core::{Event, midi::MidiMessage};

/// Plays a held-note snapshot on a rational sample clock. State survives block boundaries.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    rate: (u32, u32),
    timing: Option<(u16, Division)>,
    whole: u32,
    remainder: u64,
    divisor: u64,
    carry: u64,
    until_step: u32,
    until_off: u32,
    active: Option<u8>,
    phrase: Option<u32>,
    step: usize,
    random: u32,
}

impl Clock {
    /// Construct for a nonzero rational sample rate in 1,000..=768,000 Hz.
    #[must_use]
    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        let den = u64::from(denominator);
        if den == 0 || !(den * 1_000..=den * 768_000).contains(&u64::from(numerator)) {
            return None;
        }
        Some(Self {
            rate: (numerator, denominator),
            timing: None,
            whole: 0,
            remainder: 0,
            divisor: 1,
            carry: 0,
            until_step: 0,
            until_off: 0,
            active: None,
            phrase: None,
            step: 0,
            random: 0x4152_5021,
        })
    }

    /// Currently gated output pitch.
    #[must_use]
    pub const fn sounding(&self) -> Option<u8> {
        self.active
    }

    /// Release the output and restart the next chord on its first step.
    ///
    /// # Errors
    /// A rejected note-off stays pending for the next call.
    pub fn stop<E>(&mut self, mut release: impl FnMut(u8) -> Result<(), E>) -> Result<(), E> {
        if let Some(key) = self.active {
            release(key)?;
        }
        self.active = None;
        self.phrase = None;
        self.until_step = 0;
        self.until_off = 0;
        self.carry = 0;
        self.step = 0;
        Ok(())
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The constructor bounds the sample rate; the normalized tempo and subdivision are positive."
    )]
    fn set_timing(&mut self, bpm: u16, division: Division) {
        let bpm = bpm.clamp(30, 300);
        if self.timing == Some((bpm, division)) {
            return;
        }
        let numerator = u64::from(self.rate.0) * 60;
        self.divisor = u64::from(self.rate.1) * u64::from(bpm) * division as u64;
        self.whole =
            u32::try_from(numerator / self.divisor).expect("at most two seconds at 768 kHz");
        self.remainder = numerator % self.divisor;
        self.carry = 0;
        self.timing = Some((bpm, division));
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A nonempty chord spans at most four passes of 128 notes."
    )]
    fn next_index(&self, mode: Mode, count: usize) -> (usize, usize) {
        let cycle = if mode == Mode::UpDown && count > 1 {
            (count - 1) * 2
        } else {
            count
        };
        let i = self.step % cycle;
        let index = match mode {
            Mode::Down => count - 1 - i,
            Mode::UpDown if i >= count => cycle - i,
            Mode::Random => self.random as usize % count,
            _ => i,
        };
        (index, (i + 1) % cycle)
    }

    /// Emit events at offsets within this block. An empty chord immediately
    /// releases its output. The first step of a new chord sounds at frame zero.
    /// Rate and gate edits apply to the next step. A zero-frame call delivers
    /// due events without advancing the clock.
    ///
    /// # Errors
    /// Returns the sink's error. A rejected release is retried before the next note-on.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::missing_panics_doc,
        clippy::unwrap_in_result,
        reason = "The index is within a nonempty chord and the elapsed span fits a single step (at most two seconds at 768 kHz)."
    )]
    pub fn render<E>(
        &mut self,
        chord: &Chord,
        config: Config,
        bpm: u16,
        frames: usize,
        mut emit: impl FnMut(Event<MidiMessage>) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut send = |frame, message| {
            emit(Event {
                frame,
                port: 0,
                message,
            })
        };
        if chord.is_empty() || self.phrase != Some(chord.phrase) {
            self.stop(|key| send(0, MidiMessage::Off(key)))?;
            if chord.is_empty() {
                return Ok(());
            }
            self.phrase = Some(chord.phrase);
        }
        self.set_timing(bpm, config.division);
        let octaves = config.octaves.clamp(1, 4);
        let mut frame = 0;
        loop {
            if self.until_off == 0
                && let Some(key) = self.active
            {
                send(frame, MidiMessage::Off(key))?;
                self.active = None;
            }
            if self.until_step == 0 {
                let count = (0..octaves).map(|o| chord.count(o)).sum();
                let (index, next_step) = self.next_index(config.mode, count);
                let note = chord
                    .note(index, octaves, config.mode == Mode::AsPlayed)
                    .expect("index within the playable chord");
                send(frame, MidiMessage::On(note))?;
                self.active = Some(note.number);
                self.step = next_step;
                if config.mode == Mode::Random {
                    self.random ^= self.random << 13;
                    self.random ^= self.random >> 17;
                    self.random ^= self.random << 5;
                }
                self.carry += self.remainder;
                self.until_step = self.whole;
                if self.carry >= self.divisor {
                    self.carry -= self.divisor;
                    self.until_step += 1;
                }
                self.until_off =
                    (self.until_step * u32::from(config.gate.clamp(1, 100)) / 100).max(1);
            }
            let next = if self.active.is_some() {
                self.until_off
            } else {
                self.until_step
            };
            let span = (next as usize).min(frames - frame);
            let elapsed = u32::try_from(span).expect("bounded by one step");
            self.until_off = self.until_off.saturating_sub(elapsed);
            self.until_step -= elapsed;
            frame += span;
            if frame == frames {
                return Ok(());
            }
        }
    }
}
