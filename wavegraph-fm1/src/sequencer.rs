//! FM-1 master transport and the audio-clocked 909 pattern player.

use crate::instrument::Message;
use wavegraph::{Event, Port, Process, ProcessContext};
use wavegraph_909::{DEMO_PATTERN, Samples, Tr909};
use wavegraph_sample::I24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Transport {
    pub(super) bpm: u16,
    // Every edge advances the generation, so even a stop/start between audio
    // blocks restarts the pattern. Odd generations are playing.
    generation: u32,
}

impl Transport {
    pub(super) const INITIAL: Self = Self {
        bpm: 126,
        generation: 0,
    };

    pub(super) fn playing(self) -> bool {
        self.generation & 1 != 0
    }

    pub(super) fn toggle(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SeqPage {
    Pattern,
    Tempo,
}

/// The stock sequencer puts tempo on page two, knob three.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Sequencer {
    pub(super) page: Option<SeqPage>,
    pub(super) transport: Transport,
}

impl Sequencer {
    pub(super) const INITIAL: Self = Self {
        page: None,
        transport: Transport::INITIAL,
    };

    pub(super) fn toggle_page(&mut self) {
        self.page = if self.page.is_none() {
            Some(SeqPage::Pattern)
        } else {
            None
        };
    }

    pub(super) fn turn(&mut self, select: i8, knob3: i8) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if select > 0 {
            *page = SeqPage::Tempo;
        } else if select < 0 {
            *page = SeqPage::Pattern;
        }
        if *page == SeqPage::Tempo {
            self.transport.bpm = self
                .transport
                .bpm
                .saturating_add_signed(i16::from(knob3))
                .clamp(30, 300);
        }
    }
}

struct StepClock {
    threshold: u64,
    rate_denominator: u32,
    phase: u64,
    increment: u64,
    next: usize,
}

impl StepClock {
    fn new(numerator: u32, denominator: u32) -> Self {
        // One sixteenth lasts 15/BPM seconds. Keep the fractional frame across
        // steps and tempo edits, even at the FM-1's 24 MHz / 544 frame rate.
        let threshold = 15 * u64::from(numerator);
        Self {
            threshold,
            rate_denominator: denominator,
            phase: threshold,
            increment: 0,
            next: 0,
        }
    }

    fn restart(&mut self) {
        self.phase = self.threshold;
        self.next = 0;
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A u16 times a u32 fits in u64."
    )]
    fn set_tempo(&mut self, bpm: u16) {
        self.increment = u64::from(bpm) * u64::from(self.rate_denominator);
    }

    /// Position of the last rendered frame within the quarter note (1/256).
    /// Each kick starts at zero, using the same sub-frame remainder as the triggers.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "The elapsed part is below one nonzero sixteenth; the step is modulo four."
    )]
    fn beat_phase(&self) -> u8 {
        let step = (self.next.wrapping_sub(1) % 4) as u8;
        let elapsed = self.phase - self.increment;
        (step << 6) | ((elapsed * 64 / self.threshold) as u8)
    }

    #[inline]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "At audio rates phase stays below threshold + increment, and next is a pattern index."
    )]
    fn span(&mut self, available: usize) -> (Option<usize>, usize) {
        debug_assert!(available != 0 && self.increment != 0);
        let step = if self.phase >= self.threshold {
            self.phase -= self.threshold;
            let step = self.next;
            self.next = (step + 1) % DEMO_PATTERN.len();
            Some(step)
        } else {
            None
        };
        let left = self.threshold - self.phase;
        // Most DMA blocks lie wholly within one step. Divide only at a boundary.
        let frames = if (available as u64).saturating_mul(self.increment) <= left {
            available
        } else {
            usize::try_from(left.div_ceil(self.increment)).expect("bounded by available frames")
        };
        self.phase += (frames as u64) * self.increment;
        (step, frames)
    }

    #[cfg(test)]
    fn tick(&mut self) -> Option<usize> {
        self.span(1).0
    }
}

pub(super) struct DrumMachine {
    kit: Tr909<'static>,
    clock: StepClock,
    transport: Transport,
}

impl DrumMachine {
    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            kit: Tr909::with_sample_rate(Samples::X0XPROC1, rate.0, rate.1)
                .expect("909 clock and PCM"),
            clock: StepClock::new(rate.0, rate.1),
            transport: Transport::INITIAL,
        }
    }

    fn update(&mut self, transport: Transport) {
        if self.transport.generation != transport.generation {
            self.kit.stop();
            self.clock.restart();
        }
        self.clock.set_tempo(transport.bpm);
        self.transport = transport;
    }

    fn render(&mut self, out: &mut [I24]) {
        if !self.transport.playing() {
            out.fill(I24::default());
            return;
        }
        let mut remaining = out;
        while !remaining.is_empty() {
            let (step, frames) = self.clock.span(remaining.len());
            if let Some(step) = step {
                for &voice in DEMO_PATTERN[step] {
                    self.kit.trigger(voice);
                }
            }
            let (span, tail) = remaining.split_at_mut(frames);
            for sample in span {
                *sample = self.kit.next_mix_frame();
            }
            remaining = tail;
        }
    }
}

impl Process<Message, I24> for DrumMachine {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal, Port::Message]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        let out = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            if let Message::Transport(transport) = event.message {
                self.render(&mut out[start..event.frame]);
                self.update(transport);
                start = event.frame;
            }
        }
        self.render(&mut out[start..cx.frames]);
        if let Some(frame) = cx.frames.checked_sub(1) {
            let phase = self.transport.playing().then(|| self.clock.beat_phase());
            let _ = (cx.emit)(Event {
                frame,
                port: 1,
                message: Message::BeatPhase(phase),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_page_two_knob_three_edits_one_global_bpm() {
        let mut seq = Sequencer::INITIAL;
        assert!(!seq.transport.playing());
        seq.turn(1, -35);
        seq.toggle_page();
        assert_eq!(seq.page, Some(SeqPage::Pattern));
        seq.turn(0, -35);
        assert_eq!(seq.transport.bpm, 126);
        seq.turn(1, 2);
        assert_eq!(seq.page, Some(SeqPage::Tempo));
        assert_eq!(seq.transport.bpm, 128);
        seq.transport.toggle();
        seq.turn(0, -128);
        assert_eq!(seq.transport.bpm, 30);
        for _ in 0..3 {
            seq.turn(0, 127);
        }
        assert_eq!(seq.transport.bpm, 300);
        seq.turn(-1, -50);
        seq.toggle_page();
        assert_eq!(seq.page, None);
        assert!(seq.transport.playing());
        assert_eq!(seq.transport.bpm, 300);
    }

    #[test]
    fn sixteenths_do_not_drift_at_the_fractional_fm1_clock() {
        let mut clock = StepClock::new(24_000_000, 544);
        clock.set_tempo(137);
        let mut frame = 0_u64;
        for n in 0..97_u64 {
            while clock.tick().is_none() {
                frame += 1;
            }
            assert_eq!(frame, (n * 360_000_000).div_ceil(137 * 544));
            frame += 1;
        }
    }

    #[test]
    fn pulse_crosses_once_per_kick_and_keeps_phase_when_tempo_changes() {
        let mut clock = StepClock::new(48_000, 1);
        clock.set_tempo(120);
        // Tick in 1/48,000-beat units: two per frame at 120 BPM and three at 180.
        let mut beat_units = 0_u32;
        for frame in 0..64_000 {
            if frame == 9000 {
                clock.set_tempo(180);
            }
            let step = clock.tick();
            let expected = u8::try_from(beat_units * 256 / 48_000).expect("within one beat");
            assert_eq!(clock.beat_phase(), expected);
            if step.is_some_and(|step| step % 4 == 0) {
                assert_eq!(clock.beat_phase(), 0);
            }
            beat_units = (beat_units + if frame < 9000 { 2 } else { 3 }) % 48_000;
        }
        clock.restart();
        assert_eq!(clock.tick(), Some(0));
        assert_eq!(clock.beat_phase(), 0);
    }

    #[test]
    fn variable_dma_spans_match_the_single_frame_clock() {
        let mut block = StepClock::new(24_000_000, 544);
        let mut reference = StepClock::new(24_000_000, 544);
        for n in 0..2000 {
            let bpm = if n < 1013 { 126 } else { 300 };
            block.set_tempo(bpm);
            reference.set_tempo(bpm);
            let mut left = [128, 65, 1, 209][n % 4];
            while left != 0 {
                let (step, count) = block.span(left);
                assert!((1..=left).contains(&count));
                assert_eq!(reference.tick(), step);
                for _ in 1..count {
                    assert_eq!(reference.tick(), None);
                }
                assert_eq!(block.beat_phase(), reference.beat_phase());
                left -= count;
            }
        }
    }

    #[test]
    fn a_tempo_edit_keeps_the_elapsed_fraction_of_the_step() {
        let mut clock = StepClock::new(48_000, 1);
        clock.set_tempo(120);
        assert_eq!(clock.tick(), Some(0));
        for _ in 1..3000 {
            assert_eq!(clock.tick(), None);
        }
        clock.set_tempo(240);
        for _ in 0..1500 {
            assert_eq!(clock.tick(), None);
        }
        assert_eq!(clock.tick(), Some(1));
    }

    #[test]
    fn frame_timed_transport_mutes_tails_and_restarts_on_the_downbeat() {
        let mut drums = DrumMachine::new((44_100, 1));
        let mut play = Transport::INITIAL;
        play.toggle();
        let mut stop = play;
        stop.toggle();
        let mut out = [I24::default(); 256];
        drums.process(&mut ProcessContext {
            frames: 256,
            inputs: &[],
            outputs: &mut [&mut out],
            events: &[(11, play), (100, stop)].map(|(frame, state)| Event {
                frame,
                port: 0,
                message: Message::Transport(state),
            }),
            emit: &mut Err,
        });
        assert!(out[..11].iter().chain(&out[100..]).all(|s| s.inner() == 0));
        assert!(out[11..100].iter().any(|s| s.inner() != 0));
        stop.toggle();
        drums.update(stop);
        drums.render(&mut out[..40]);
        assert_eq!(drums.clock.next, 1);
        let mut new_play = stop;
        new_play.toggle();
        new_play.toggle();
        drums.update(new_play);
        assert_eq!(drums.clock.next, 0);
        drums.render(&mut out[..1]);
        assert_eq!(drums.clock.next, 1);
    }
}
