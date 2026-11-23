//! FM-1 master transport and one audio-clocked player for all factory patterns.

use crate::instrument::{Message, Part};
use wavegraph::{Event, Port, Process, ProcessContext, bytecode::Error};
use wavegraph_303::Message as BassMessage;
use wavegraph_909::{DEMO_PATTERN, Samples, Tr909};
use wavegraph_monosynth::Message as MonoMessage;
use wavegraph_organ::Message as OrganMessage;
use wavegraph_sample::I24;

pub(super) mod pattern;

const SUBSTEPS: usize = pattern::STEPS * 8;
const AUTOMATION_FRAMES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Transport {
    pub(super) bpm: u16,
    // Every edge advances the generation, so even a stop/start between audio
    // blocks restarts the pattern. Odd generations are playing.
    generation: u32,
}

impl Transport {
    pub(super) const INITIAL: Self = Self {
        bpm: 122,
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
pub(super) enum Track {
    Mono,
    Organ,
    Bass,
    Drums,
}

impl Track {
    pub(super) const ALL: [Self; 4] = [Self::Mono, Self::Organ, Self::Bass, Self::Drums];

    pub(super) const fn part(self) -> Option<Part> {
        match self {
            Self::Mono => Some(Part::Mono),
            Self::Organ => Some(Part::Organ),
            Self::Bass => Some(Part::Bass),
            Self::Drums => None,
        }
    }
}

impl From<Part> for Track {
    fn from(part: Part) -> Self {
        match part {
            Part::Mono => Self::Mono,
            Part::Organ => Self::Organ,
            Part::Bass => Self::Bass,
        }
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
    pub(super) track: Track,
    pub(super) bar: u8,
    pub(super) enabled: [bool; 4],
}

impl Sequencer {
    pub(super) const INITIAL: Self = Self {
        page: None,
        transport: Transport::INITIAL,
        track: Track::Mono,
        bar: 0,
        // The optional counterline starts muted, leaving room for chords and acid.
        enabled: [false, true, true, true],
    };

    pub(super) fn toggle_page(&mut self) {
        self.page = if self.page.is_none() {
            Some(SeqPage::Pattern)
        } else {
            None
        };
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A track/bar index plus an i8 delta fits i16."
    )]
    pub(super) fn turn_track(&mut self, detents: i8) {
        let index = (i16::from(self.track as u8) + i16::from(detents)).rem_euclid(4);
        self.track = Track::ALL[usize::try_from(index).expect("four track indices")];
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "A bar index plus an i8 delta fits i16."
    )]
    pub(super) fn turn_bar(&mut self, detents: i8) {
        if !self.transport.playing() {
            self.bar = u8::try_from((i16::from(self.bar) + i16::from(detents)).rem_euclid(4))
                .expect("four bars");
        }
    }

    pub(super) fn toggle_track(&mut self) {
        let enabled = &mut self.enabled[self.track as usize];
        *enabled = !*enabled;
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

/// Last rendered sample's position in the shared four-bar loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub(super) step: u8,
    pub(super) beat_phase: u8,
}

// One tick is an eighth of a sixteenth, providing short gates without another
// clock. The rational remainder survives tempo edits and DMA block boundaries.
struct StepClock {
    threshold: u32,
    rate_denominator: u32,
    phase: u32,
    increment: u32,
    next: usize,
}

impl StepClock {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Nonzero denominator is checked before division; two positive integers have nonzero gcd."
    )]
    fn new(numerator: u32, denominator: u32) -> Self {
        assert!(
            denominator != 0 && numerator / denominator >= 1000,
            "the sequencer requires an audio sample rate of at least 1 kHz"
        );
        // Reduce the rational clock before forming phase units. In particular,
        // the FM-1's 24 MHz / 544 becomes 750,000 / 17 without losing precision.
        // Keep the hot path on native-width arithmetic on this 32-bit target.
        let divisor = gcd(numerator, denominator);
        let rate_denominator = denominator / divisor;
        let threshold = (numerator / divisor)
            .checked_mul(15)
            .expect("sequencer sample-rate numerator fits the 32-bit clock");
        let maximum_increment = rate_denominator
            .checked_mul(300 * 8)
            .expect("sequencer sample-rate denominator fits the 32-bit clock");
        assert!(
            threshold.checked_add(maximum_increment).is_some(),
            "sequencer phase plus one sample must fit the 32-bit clock"
        );
        Self {
            threshold,
            rate_denominator,
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
        reason = "The constructor checked the maximum increment at 300 BPM."
    )]
    fn set_tempo(&mut self, bpm: u16) {
        self.increment = u32::from(bpm.clamp(30, 300)) * self.rate_denominator * 8;
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Subtraction is guarded and the next tick wraps within the pattern."
    )]
    fn take_tick(&mut self) -> Option<usize> {
        if self.phase < self.threshold {
            return None;
        }
        self.phase -= self.threshold;
        let tick = self.next;
        self.next = (tick + 1) % SUBSTEPS;
        Some(tick)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Every span ends at its next tick; the constructor bounded threshold + increment."
    )]
    fn advance(&mut self, available: usize) -> usize {
        debug_assert!(available != 0 && self.increment != 0);
        let left = self.threshold - self.phase;
        let frames = match u32::try_from(available)
            .ok()
            .and_then(|frames| frames.checked_mul(self.increment))
        {
            Some(advance) if advance <= left => available,
            _ => {
                usize::try_from(left.div_ceil(self.increment)).expect("bounded by available frames")
            }
        };
        self.phase += u32::try_from(frames).expect("bounded by one tick") * self.increment;
        frames
    }

    /// Loop position and within-step fraction at the current or last rendered frame.
    /// Quarter-note phase shares the same sub-frame remainder as drum triggers.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Normalized tick positions and their fractions stay within 0..256."
    )]
    fn position(&self, last_frame: bool) -> (Position, u8) {
        let tick = self.next.wrapping_sub(1) % SUBSTEPS;
        let phase = self.phase - if last_frame { self.increment } else { 0 };
        let fine = fraction_32(phase, self.threshold);
        let fraction = (tick % 8) * 32 + usize::from(fine);
        let beat_phase = (tick % 32) * 8 + usize::from(fine >> 2);
        (
            Position {
                step: (tick / 8) as u8,
                beat_phase: beat_phase as u8,
            },
            fraction as u8,
        )
    }

    #[cfg(test)]
    fn tick(&mut self) -> Option<usize> {
        let tick = self.take_tick();
        self.advance(1);
        tick
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "The remainder divisor is guarded by the loop condition."
)]
fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Exact floor(32 * phase / threshold), without wide arithmetic. Real audio
/// clocks use the fast path; the fallback also supports large reduced ratios.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "phase is below threshold, and each doubled remainder stays below threshold."
)]
fn fraction_32(mut phase: u32, threshold: u32) -> u8 {
    if let Some(scaled) = phase.checked_mul(32) {
        return u8::try_from(scaled / threshold).expect("one substep fraction");
    }
    let mut fraction = 0;
    for _ in 0..5 {
        fraction <<= 1;
        if phase >= threshold - phase {
            phase -= threshold - phase;
            fraction |= 1;
        } else {
            phase += phase;
        }
    }
    fraction
}

/// One audio clock for notes, gates, drum strikes, automation and the playhead.
/// The host supplies per-track enable/ownership and stores emitted events in its
/// bounded graph queues. A full queue is an explicit error, never a dropped off.
pub(super) struct PatternPlayer {
    clock: StepClock,
    transport: Transport,
    enabled: [bool; 4],
    held: [&'static [u8]; 3],
    off_at: [Option<usize>; 3],
    bass_slide: bool,
    automation_left: usize,
    position: Option<Position>,
}

impl PatternPlayer {
    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            clock: StepClock::new(rate.0, rate.1),
            transport: Transport::INITIAL,
            enabled: [false; 4],
            held: [&[]; 3],
            off_at: [None; 3],
            bass_slide: false,
            automation_left: 0,
            position: None,
        }
    }

    pub(super) fn position(&self) -> Option<Position> {
        self.position
    }

    #[cfg(test)]
    fn automation(&self) -> Option<pattern::Automation> {
        self.position
            .filter(|_| self.enabled[Track::Bass as usize])
            .map(|_| {
                let (position, fraction) = self.clock.position(true);
                pattern::automation(usize::from(position.step), fraction)
            })
    }

    fn release(
        &mut self,
        track: Track,
        frame: usize,
        emit: &mut impl FnMut(Track, Event<Message>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let i = track as usize;
        for &note in self.held[i] {
            let message = match track {
                Track::Mono => Message::Mono(MonoMessage::NoteOff(note)),
                Track::Organ => Message::Organ(OrganMessage::NoteOff(note)),
                Track::Bass => Message::Bass(BassMessage::NoteOff(note)),
                Track::Drums => unreachable!("only synth tracks have gates"),
            };
            emit(track, event(frame, message))?;
        }
        self.held[i] = &[];
        self.off_at[i] = None;
        if track == Track::Bass {
            self.bass_slide = false;
        }
        Ok(())
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Tick and bounded gate duration fit usize before the four-bar wrap."
    )]
    fn on_tick(
        &mut self,
        tick: usize,
        frame: usize,
        emit: &mut impl FnMut(Track, Event<Message>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        for track in [Track::Mono, Track::Organ, Track::Bass] {
            if self.off_at[track as usize] == Some(tick) {
                self.release(track, frame, emit)?;
            }
        }
        if !tick.is_multiple_of(8) {
            return Ok(());
        }
        let step = tick / 8;
        for track in [Track::Mono, Track::Organ, Track::Bass] {
            let i = track as usize;
            if !self.enabled[i] {
                continue;
            }
            let data = pattern::step(track, step);
            if data.notes.is_empty() {
                // Invalid/unpaired outgoing slides cannot leave a gate hanging.
                if track == Track::Bass && self.bass_slide {
                    self.release(track, frame, emit)?;
                }
                continue;
            }
            let slide = track == Track::Bass && self.bass_slide && !self.held[i].is_empty();
            if !slide {
                self.release(track, frame, emit)?;
            }
            for &note in data.notes {
                let message = match track {
                    Track::Mono => Message::Mono(MonoMessage::NoteOn {
                        note,
                        velocity: data.velocity,
                    }),
                    Track::Organ => Message::Organ(OrganMessage::NoteOn {
                        note,
                        velocity: data.velocity,
                    }),
                    Track::Bass => Message::Bass(BassMessage::NoteOn {
                        note,
                        accent: data.velocity >= 96,
                        slide,
                    }),
                    Track::Drums => unreachable!("drums have their own pattern"),
                };
                emit(track, event(frame, message))?;
            }
            self.held[i] = data.notes;
            if track == Track::Bass && data.slide {
                self.bass_slide = true;
                self.off_at[i] = None;
            } else {
                if track == Track::Bass {
                    self.bass_slide = false;
                }
                self.off_at[i] = Some((tick + usize::from(data.gate.max(1))) % SUBSTEPS);
            }
        }
        if self.enabled[Track::Drums as usize] {
            for &voice in DEMO_PATTERN[step % DEMO_PATTERN.len()] {
                emit(
                    Track::Drums,
                    event(frame, Message::Drum(DrumMessage::Trigger(voice))),
                )?;
            }
        }
        Ok(())
    }

    /// Emit frame offsets relative to this block. `enabled` includes both the
    /// pattern mute state and whether live keys currently own that instrument.
    pub(super) fn render(
        &mut self,
        frames: usize,
        transport: Transport,
        enabled: [bool; 4],
        mut emit: impl FnMut(Track, Event<Message>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if frames == 0 {
            return Ok(());
        }
        let restart = self.transport.generation != transport.generation;
        if restart {
            for track in [Track::Mono, Track::Organ, Track::Bass] {
                self.release(track, 0, &mut emit)?;
            }
            emit(Track::Drums, event(0, Message::Drum(DrumMessage::Stop)))?;
            self.clock.restart();
            self.automation_left = 0;
        } else {
            for track in [Track::Mono, Track::Organ, Track::Bass] {
                if self.enabled[track as usize] && !enabled[track as usize] {
                    self.release(track, 0, &mut emit)?;
                }
            }
            if self.enabled[Track::Drums as usize] && !enabled[Track::Drums as usize] {
                emit(Track::Drums, event(0, Message::Drum(DrumMessage::Stop)))?;
            }
        }
        if !self.enabled[Track::Bass as usize] && enabled[Track::Bass as usize] {
            self.automation_left = 0;
        }
        self.enabled = enabled;
        self.transport = transport;
        self.clock.set_tempo(transport.bpm);
        if !transport.playing() {
            self.position = None;
            return Ok(());
        }
        let mut frame = 0;
        while frame < frames {
            if let Some(tick) = self.clock.take_tick() {
                self.on_tick(tick, frame, &mut emit)?;
            }
            if self.automation_left == 0 {
                if enabled[Track::Bass as usize] {
                    let (position, fraction) = self.clock.position(false);
                    let [cutoff, resonance, drive] =
                        pattern::automation_values(usize::from(position.step), fraction);
                    for message in [
                        BassMessage::CutoffHz(cutoff),
                        BassMessage::Resonance(resonance / 100.0),
                        BassMessage::OverdriveDb(drive),
                    ] {
                        emit(Track::Bass, event(frame, Message::Bass(message)))?;
                    }
                }
                self.automation_left = AUTOMATION_FRAMES;
            }
            let available = frames.saturating_sub(frame).min(self.automation_left);
            let count = self.clock.advance(available);
            frame = frame.saturating_add(count);
            self.automation_left = self.automation_left.saturating_sub(count);
        }
        self.position = Some(self.clock.position(true).0);
        Ok(())
    }
}

fn event(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DrumMessage {
    Trigger(wavegraph_909::Voice),
    Stop,
}

/// The drum renderer has no private transport; it consumes the shared player's
/// frame-stamped strikes just like every pitched instrument.
pub(super) struct DrumMachine {
    kit: Tr909<'static>,
}

impl DrumMachine {
    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            kit: Tr909::with_sample_rate(Samples::X0XPROC1, rate.0, rate.1)
                .expect("909 clock and PCM"),
        }
    }

    fn render(&mut self, out: &mut [I24]) {
        for sample in out {
            // Reserve mix headroom for the resonant bass and organ chord peaks.
            *sample = I24::new(self.kit.next_mix_frame::<I24>().inner() >> 1)
                .expect("half of a Q23 sample is representable");
        }
    }
}

impl Process<Message, I24> for DrumMachine {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        let out = &mut cx.outputs[0];
        let mut start = 0;
        for event in cx.events {
            if let Message::Drum(message) = event.message {
                self.render(&mut out[start..event.frame]);
                match message {
                    DrumMessage::Trigger(voice) => self.kit.trigger(voice),
                    DrumMessage::Stop => self.kit.stop(),
                }
                start = event.frame;
            }
        }
        self.render(&mut out[start..cx.frames]);
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use alloc::vec::Vec;

    fn playing() -> Transport {
        let mut transport = Transport::INITIAL;
        transport.toggle();
        transport
    }

    fn render(
        player: &mut PatternPlayer,
        frames: usize,
        transport: Transport,
        enabled: [bool; 4],
    ) -> Vec<(Track, Event<Message>)> {
        let mut events = Vec::new();
        player
            .render(frames, transport, enabled, |track, event| {
                events.push((track, event));
                Ok(())
            })
            .expect("unbounded host event sink");
        events
    }

    #[test]
    fn panel_browses_four_tracks_and_edits_one_global_bpm() {
        let mut seq = Sequencer::INITIAL;
        assert_eq!(seq.enabled, [false, true, true, true]);
        seq.turn_track(-1);
        assert_eq!(seq.track, Track::Drums);
        seq.toggle_track();
        assert!(!seq.enabled[3]);
        seq.turn_bar(-1);
        assert_eq!(seq.bar, 3);
        seq.turn(1, 127);
        assert_eq!(seq.transport.bpm, 122);
        seq.toggle_page();
        seq.turn(0, 127);
        assert_eq!(seq.transport.bpm, 122);
        seq.turn(1, 2);
        assert_eq!(seq.transport.bpm, 124);
        seq.transport.toggle();
        seq.turn_bar(1);
        assert_eq!(seq.bar, 3);
        seq.turn(0, -128);
        assert_eq!(seq.transport.bpm, 30);
        for _ in 0..3 {
            seq.turn(0, 127);
        }
        assert_eq!(seq.transport.bpm, 300);
        seq.turn(-1, 0);
        assert_eq!(seq.page, Some(SeqPage::Pattern));
        seq.toggle_page();
        assert_eq!(seq.page, None);
        assert!(seq.transport.playing());
        for part in [Part::Mono, Part::Organ, Part::Bass] {
            assert_eq!(Track::from(part).part(), Some(part));
        }
        assert_eq!(Track::Drums.part(), None);
    }

    #[test]
    fn rational_substeps_and_gates_do_not_drift_at_the_fm1_clock() {
        let mut clock = StepClock::new(24_000_000, 544);
        clock.set_tempo(137);
        let mut frame = 0_u64;
        for n in 0..1025_u64 {
            let tick = loop {
                if let Some(tick) = clock.tick() {
                    break tick;
                }
                frame += 1;
            };
            assert_eq!(frame, (n * 360_000_000).div_ceil(137 * 544 * 8));
            assert_eq!(
                tick,
                usize::try_from(n).expect("small tick index") % SUBSTEPS
            );
            frame += 1;
        }
    }

    #[test]
    fn native_width_phase_fraction_matches_wide_reference_at_large_ratios() {
        for threshold in [1, 3, 11_250_000, u32::MAX - 40_800, u32::MAX] {
            for phase in [0, threshold / 3, threshold / 2, threshold - 1] {
                assert_eq!(
                    u64::from(fraction_32(phase, threshold)),
                    u64::from(phase) * 32 / u64::from(threshold)
                );
            }
        }
        let mut clock = StepClock::new(24_000_000, 544);
        clock.set_tempo(122);
        assert_eq!(clock.threshold, 11_250_000);
        assert_eq!(clock.rate_denominator, 17);
        assert_eq!(clock.increment, 16_592);
    }

    #[test]
    fn quarter_note_phase_keeps_time_across_tempo_edits() {
        let mut clock = StepClock::new(48_000, 1);
        clock.set_tempo(120);
        let mut beat_units = 0_u32;
        for frame in 0..64_000 {
            if frame == 9000 {
                clock.set_tempo(180);
            }
            let tick = clock.tick();
            let expected = u8::try_from(beat_units * 256 / 48_000).expect("one quarter note");
            assert_eq!(clock.position(true).0.beat_phase, expected);
            if tick.is_some_and(|tick| tick % 32 == 0) {
                assert_eq!(clock.position(true).0.beat_phase, 0);
            }
            beat_units = (beat_units + if frame < 9000 { 2 } else { 3 }) % 48_000;
        }
    }

    #[test]
    fn dma_partition_does_not_move_notes_gates_or_automation() {
        fn collect(blocks: &[usize]) -> Vec<(Track, Event<Message>)> {
            let mut player = PatternPlayer::new((24_000_000, 544));
            let mut result = Vec::new();
            let mut offset = 0;
            let mut block = 0;
            while offset < 200_000 {
                let size = blocks[block % blocks.len()].min(200_000 - offset);
                for (track, mut event) in render(&mut player, size, playing(), [true; 4]) {
                    event.frame += offset;
                    result.push((track, event));
                }
                offset += size;
                block += 1;
            }
            result
        }
        assert_eq!(collect(&[128]), collect(&[65, 1, 209, 93]));
    }

    #[test]
    fn outgoing_slides_keep_the_gate_and_the_following_step_releases_it() {
        let mut player = PatternPlayer::new((48_000, 1));
        let transport = Transport {
            bpm: 120,
            ..playing()
        };
        let events = render(&mut player, 49_000, transport, [false, false, true, false]);
        let notes: Vec<_> = events
            .into_iter()
            .filter_map(|(_, e)| match e.message {
                Message::Bass(BassMessage::NoteOn {
                    note,
                    accent,
                    slide,
                }) => Some((e.frame, Some((note, accent, slide)))),
                Message::Bass(BassMessage::NoteOff(_)) => Some((e.frame, None)),
                _ => None,
            })
            .collect();
        assert!(notes.contains(&(36_000, Some((45, false, false)))));
        assert!(notes.contains(&(42_000, Some((48, false, true)))));
        assert!(
            !notes
                .iter()
                .any(|&(frame, note)| (36_000..42_000).contains(&frame) && note.is_none())
        );
        assert!(notes.contains(&(45_750, None)));
        assert!(notes.contains(&(0, Some((38, true, false)))));
    }

    #[test]
    fn stop_mute_and_fast_restart_release_only_owned_sequence_gates() {
        let mut player = PatternPlayer::new((48_000, 1));
        let play = Transport {
            bpm: 120,
            ..playing()
        };
        render(&mut player, 1, play, [false, true, true, true]);
        let muted = render(&mut player, 128, play, [false, true, false, false]);
        assert!(
            muted
                .iter()
                .any(|(_, event)| event.message == Message::Bass(BassMessage::NoteOff(38)))
        );
        assert!(
            muted
                .iter()
                .any(|(_, event)| event.message == Message::Drum(DrumMessage::Stop))
        );
        assert_eq!(player.automation(), None);
        let mut restart = play;
        restart.toggle();
        restart.toggle();
        let events = render(&mut player, 1, restart, [false, true, true, true]);
        assert_eq!(
            player.position(),
            Some(Position {
                step: 0,
                beat_phase: 0
            })
        );
        assert!(events.iter().any(|(_, event)| matches!(
            event.message,
            Message::Bass(BassMessage::NoteOn { note: 38, .. })
        )));
        restart.toggle();
        let stopped = render(&mut player, 128, restart, [false, true, true, true]);
        assert_eq!(player.position(), None);
        assert_eq!(player.automation(), None);
        assert!(
            stopped
                .iter()
                .any(|(_, event)| event.message == Message::Bass(BassMessage::NoteOff(38)))
        );
        assert!(render(&mut player, 128, restart, [true; 4]).is_empty());
    }

    #[test]
    fn automation_is_bounded_interpolated_and_continuous_at_the_loop() {
        let mut previous = pattern::automation_values(0, 0);
        let mut changes = [false; 3];
        for step in 0..pattern::STEPS {
            for fraction in 0..=255 {
                let values = pattern::automation_values(step, fraction);
                assert!((150.0..=1800.0).contains(&values[0]));
                assert!((72.0..=94.0).contains(&values[1]));
                assert!((12.0..=28.0).contains(&values[2]));
                for lane in 0..3 {
                    changes[lane] |= previous[lane] != values[lane];
                    assert!((previous[lane] - values[lane]).abs() < 1.0);
                }
                previous = values;
            }
        }
        assert_eq!(changes, [true; 3]);
        let first = pattern::automation_values(0, 0);
        for lane in 0..3 {
            assert!((previous[lane] - first[lane]).abs() < 1.0);
        }
    }

    #[test]
    fn a_full_event_sink_is_an_explicit_failure() {
        let mut player = PatternPlayer::new((44_100, 1));
        assert_eq!(
            player.render(128, playing(), [true; 4], |_, _| Err(Error::Capacity)),
            Err(Error::Capacity)
        );
    }

    #[test]
    fn drum_trigger_and_stop_obey_the_shared_frame_offsets() {
        let mut drums = DrumMachine::new((44_100, 1));
        let mut out = [I24::default(); 256];
        drums.process(&mut ProcessContext {
            frames: 256,
            inputs: &[],
            outputs: &mut [&mut out],
            events: &[
                event(
                    11,
                    Message::Drum(DrumMessage::Trigger(wavegraph_909::Voice::Kick)),
                ),
                event(100, Message::Drum(DrumMessage::Stop)),
            ],
            emit: &mut Err,
        });
        assert!(
            out[..11]
                .iter()
                .chain(&out[100..])
                .all(|sample| sample.inner() == 0)
        );
        assert!(out[11..100].iter().any(|sample| sample.inner() != 0));
    }
}
