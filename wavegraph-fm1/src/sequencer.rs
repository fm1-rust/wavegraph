//! FM-1 master transport and one audio-clocked player for all factory patterns.

use crate::{
    instrument::{Message, Part},
    panel::{Assignment, CHANNELS, Channel, Navigation},
};
use wavegraph::{Event, Port, Process, ProcessContext, bytecode::Error};
use wavegraph_303::Message as BassMessage;
use wavegraph_909::{Samples, Tr909, Voice};
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
pub(super) enum SeqPage {
    Pattern,
    Tempo,
}

/// The stock sequencer puts tempo on page two, knob three.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Sequencer {
    pub(super) page: Option<SeqPage>,
    pub(super) transport: Transport,
    pub(super) channel: Channel,
    pub(super) bar: u8,
    pub(super) enabled: [bool; CHANNELS],
}

impl Sequencer {
    pub(super) const INITIAL: Self = Self {
        page: None,
        transport: Transport::INITIAL,
        channel: Channel::One,
        bar: 0,
        enabled: [true; CHANNELS],
    };

    pub(super) fn toggle_page(&mut self) {
        self.page = if self.page.is_none() {
            Some(SeqPage::Pattern)
        } else {
            None
        };
    }

    pub(super) fn back(&mut self) -> bool {
        if self.page == Some(SeqPage::Tempo) {
            self.page = Some(SeqPage::Pattern);
            true
        } else {
            false
        }
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
        let enabled = &mut self.enabled[self.channel.index()];
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
#[derive(Clone, Copy)]
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

#[derive(Clone, Copy)]
struct PatternGate {
    held: &'static [u8],
    off_at: Option<usize>,
    slide: bool,
}

impl PatternGate {
    const CLOSED: Self = Self {
        held: &[],
        off_at: None,
        slide: false,
    };
}

/// One audio clock for notes, gates, drum strikes, automation and the playhead.
/// Each mixer channel owns its sequence gates and can be taken over independently.
#[derive(Clone, Copy)]
pub(super) struct PatternPlayer {
    clock: StepClock,
    transport: Transport,
    assignments: [Assignment; CHANNELS],
    enabled: [bool; CHANNELS],
    gates: [PatternGate; CHANNELS],
    automation_left: usize,
    /// Clock position of the last set of 303 control messages sent to the graph.
    pub(super) automation_phase: (u8, u8),
    position: Option<Position>,
}

impl PatternPlayer {
    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            clock: StepClock::new(rate.0, rate.1),
            transport: Transport::INITIAL,
            assignments: Navigation::INITIAL.assignments,
            enabled: [false; CHANNELS],
            gates: [PatternGate::CLOSED; CHANNELS],
            automation_left: 0,
            automation_phase: (0, 0),
            position: None,
        }
    }

    pub(super) fn position(&self) -> Option<Position> {
        self.position
    }

    #[cfg(test)]
    fn automation(&self) -> Option<pattern::Automation> {
        self.position
            .filter(|_| self.enabled[Channel::Three.index()])
            .map(|_| {
                let (position, fraction) = self.clock.position(true);
                pattern::automation(Channel::Three, usize::from(position.step), fraction)
            })
    }

    fn release(
        &mut self,
        channel: Channel,
        frame: usize,
        emit: &mut impl FnMut(Channel, Event<Message>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let i = channel.index();
        for &note in self.gates[i].held {
            let message = match self.assignments[i].part {
                Part::Mono => Message::Mono(MonoMessage::NoteOff(note)),
                Part::Organ => Message::Organ(OrganMessage::NoteOff(note)),
                Part::Bass => Message::Bass(BassMessage::NoteOff(note)),
                Part::Tr808 | Part::Tr909 => unreachable!("only synth instruments hold gates"),
            };
            emit(channel, event(frame, message))?;
        }
        self.gates[i] = PatternGate::CLOSED;
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
        emit: &mut impl FnMut(Channel, Event<Message>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        for channel in Channel::ALL {
            if self.gates[channel.index()].off_at == Some(tick) {
                self.release(channel, frame, emit)?;
            }
        }
        if !tick.is_multiple_of(8) {
            return Ok(());
        }
        let step = tick / 8;
        for channel in Channel::ALL {
            let i = channel.index();
            if !self.enabled[i] {
                continue;
            }
            let part = self.assignments[i].part;
            if part.is_kit() {
                for &voice in pattern::drums(part, step) {
                    let velocity = match (part, voice) {
                        (Part::Tr808, Voice::Kick) => 96,
                        (Part::Tr808, Voice::Snare) => {
                            if step % 16 == 15 {
                                90
                            } else {
                                62
                            }
                        }
                        (Part::Tr808, Voice::ClosedHat) => 72,
                        (Part::Tr808, Voice::OpenHat) => 86,
                        _ => 127,
                    };
                    emit(
                        channel,
                        event(frame, Message::Drum(DrumMessage::Trigger(voice, velocity))),
                    )?;
                }
                continue;
            }
            let data = pattern::step(part, channel, step);
            if data.notes.is_empty() {
                if self.gates[i].slide {
                    self.release(channel, frame, emit)?;
                }
                continue;
            }
            let slide = self.gates[i].slide;
            if !slide {
                self.release(channel, frame, emit)?;
            }
            for &note in data.notes {
                let message = match part {
                    Part::Mono => Message::Mono(MonoMessage::NoteOn {
                        note,
                        velocity: data.velocity,
                    }),
                    Part::Organ => Message::Organ(OrganMessage::NoteOn {
                        note,
                        velocity: data.velocity,
                    }),
                    Part::Bass => Message::Bass(BassMessage::NoteOn {
                        note,
                        accent: data.velocity >= 96,
                        slide,
                    }),
                    Part::Tr808 | Part::Tr909 => unreachable!("drum hits were emitted above"),
                };
                emit(channel, event(frame, message))?;
            }
            let slide = part == Part::Bass && data.slide;
            self.gates[i] = PatternGate {
                held: data.notes,
                slide,
                off_at: (!slide).then(|| (tick + usize::from(data.gate.max(1))) % SUBSTEPS),
            };
        }
        Ok(())
    }

    /// Emit offsets relative to this block. `enabled` includes each channel's
    /// pattern mute and whether a live note presently owns its synth.
    pub(super) fn render(
        &mut self,
        frames: usize,
        transport: Transport,
        assignments: [Assignment; CHANNELS],
        enabled: [bool; CHANNELS],
        mut emit: impl FnMut(Channel, Event<Message>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if frames == 0 {
            return Ok(());
        }
        let restart = self.transport.generation != transport.generation;
        let mut bass = [false; CHANNELS];
        for channel in Channel::ALL {
            let i = channel.index();
            let changed = self.assignments[i] != assignments[i];
            if changed {
                // The old processor is being replaced in this block.
                self.gates[i] = PatternGate::CLOSED;
            } else if restart || (self.enabled[i] && !enabled[i]) {
                if assignments[i].part.is_kit() {
                    emit(channel, event(0, Message::Drum(DrumMessage::Stop)))?;
                } else {
                    self.release(channel, 0, &mut emit)?;
                }
            }
            bass[i] = enabled[i] && assignments[i].part == Part::Bass;
            if bass[i] && (changed || !self.enabled[i]) {
                self.automation_left = 0;
            }
        }
        if restart {
            self.clock.restart();
            self.automation_left = 0;
        }
        self.assignments = assignments;
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
                if bass.contains(&true) {
                    let (position, fraction) = self.clock.position(false);
                    self.automation_phase = (position.step, fraction);
                    for channel in Channel::ALL
                        .into_iter()
                        .filter(|channel| bass[channel.index()])
                    {
                        let [cutoff, resonance, drive] = pattern::automation_values(
                            channel,
                            usize::from(position.step),
                            fraction,
                        );
                        for message in [
                            BassMessage::CutoffHz(cutoff),
                            BassMessage::Resonance(resonance / 100.0),
                            BassMessage::OverdriveDb(drive),
                        ] {
                            emit(channel, event(frame, Message::Bass(message)))?;
                        }
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
    Trigger(Voice, u8),
    Controls808(wavegraph_808::Controls),
    Stop,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DrumLamp {
    #[default]
    Off,
    Tail,
    Strike,
}

/// Prepare both factory kits outside the audio interrupt. Each assignment takes
/// an independent copy of just its selected kit into the channel's player.
pub(super) struct DrumKits {
    tr808: wavegraph_808::Tr808,
    tr909: Tr909<'static>,
    flash_frames: u32,
}

impl DrumKits {
    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            tr808: wavegraph_808::Tr808::new(rate.0, rate.1).expect("808 clock"),
            tr909: Tr909::with_decay(Samples::X0XPROC1, rate.0, rate.1, 55)
                .expect("909 clock and PCM"),
            flash_frames: rate.0.checked_div(rate.1).expect("nonzero kit clock") / 16,
        }
    }

    pub(super) fn assign(&self, part: Part) -> DrumMachine {
        let kit = match part {
            Part::Tr808 => Kit::Tr808(self.tr808),
            Part::Tr909 => Kit::Tr909(self.tr909),
            _ => unreachable!("only drum assignments use a kit"),
        };
        DrumMachine {
            kit,
            frame: 0,
            struck_at: [0; 6],
            flash_frames: self.flash_frames,
        }
    }
}

#[derive(Clone, Copy)]
enum Kit {
    Tr808(wavegraph_808::Tr808),
    Tr909(Tr909<'static>),
}

fn voice808(voice: Voice) -> Option<wavegraph_808::Voice> {
    use wavegraph_808::Voice as V;
    Some(match voice {
        Voice::Kick => V::Kick,
        Voice::Snare => V::Snare,
        Voice::ClosedHat => V::ClosedHat,
        Voice::OpenHat => V::OpenHat,
        Voice::Clap | Voice::Ride => return None,
    })
}

impl Kit {
    fn voice_active(&self, voice: Voice) -> bool {
        match self {
            Self::Tr808(kit) => voice808(voice).is_some_and(|v| kit.voice_active(v)),
            Self::Tr909(kit) => kit.voice_active(voice),
        }
    }

    fn send(&mut self, message: DrumMessage) {
        match (self, message) {
            (Self::Tr808(kit), DrumMessage::Trigger(voice, velocity)) => {
                if let Some(v) = voice808(voice) {
                    kit.trigger(v, velocity);
                }
            }
            (Self::Tr808(kit), DrumMessage::Controls808(c)) => {
                kit.send(wavegraph_808::Message::Controls(c))
            }
            (Self::Tr808(kit), DrumMessage::Stop) => kit.send(wavegraph_808::Message::Stop),
            (Self::Tr909(kit), DrumMessage::Trigger(voice, _)) => kit.trigger(voice),
            (Self::Tr909(kit), DrumMessage::Stop) => kit.stop(),
            (Self::Tr909(_), DrumMessage::Controls808(_)) => (),
        }
    }
}

/// Both kits use the graph's frame-stamped transport and publish their own lamps.
#[derive(Clone, Copy)]
pub(super) struct DrumMachine {
    kit: Kit,
    frame: u32,
    struck_at: [u32; 6],
    flash_frames: u32,
}

impl DrumMachine {
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The weighted sum of six i16 voices fits i32 before clamping to Q23."
    )]
    fn render(&mut self, out: &mut [I24], key: &mut [I24]) {
        let kit = match &mut self.kit {
            Kit::Tr808(kit) => return kit.render_mix_kick(out, key),
            Kit::Tr909(kit) => kit,
        };
        if !Voice::ALL.into_iter().any(|v| kit.voice_active(v)) {
            out.fill(I24::default());
            key.fill(I24::default());
            return;
        }
        for (sample, key) in out.iter_mut().zip(key) {
            let [kick, snare, clap, closed, open, ride] = kit.next_voices().map(i32::from);
            *key = I24::new_unchecked(kick << 8);
            // Blend a saturated copy into the clean kick. Its early harmonics
            // carry the downbeat above the 50 Hz body on a small speaker.
            let kick = kick + kick.clamp(-4096, 4096);
            // Convert PCM16 to Q23 as we mix (256 is unity), with one final clamp.
            let sum = kick * 256 + snare * 136 + clap * 176 + closed * 90 + open * 90 + ride * 90;
            *sample = I24::new_unchecked(sum.clamp(-(1 << 23), (1 << 23) - 1));
        }
    }
}

impl Process<Message, I24> for DrumMachine {
    fn inlets(&self) -> &[Port] {
        &[Port::Message]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal, Port::Signal, Port::Message]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        let (mix, key) = cx.outputs.split_at_mut(1);
        let mut start = 0;
        for event in cx.events {
            if let Message::Drum(message) = event.message {
                self.render(
                    &mut mix[0][start..event.frame],
                    &mut key[0][start..event.frame],
                );
                self.kit.send(message);
                if let DrumMessage::Trigger(voice, _) = message {
                    self.struck_at[voice as usize] = self.frame.wrapping_add(
                        u32::try_from(event.frame).expect("frame within one audio block"),
                    );
                }
                start = event.frame;
            }
        }
        self.render(&mut mix[0][start..cx.frames], &mut key[0][start..cx.frames]);
        if let Some(last) = cx.frames.checked_sub(1) {
            self.frame = self
                .frame
                .wrapping_add(u32::try_from(cx.frames).expect("one audio block"));
            let lamps = Voice::ALL.map(|voice| {
                if !self.kit.voice_active(voice) {
                    DrumLamp::Off
                } else if self.frame.wrapping_sub(self.struck_at[voice as usize])
                    <= self.flash_frames
                {
                    DrumLamp::Strike
                } else {
                    DrumLamp::Tail
                }
            });
            let _ = (cx.emit)(Event {
                frame: last,
                port: 2,
                message: Message::DrumLamps(lamps),
            });
        }
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
    ) -> Vec<(Channel, Event<Message>)> {
        let mut events = Vec::new();
        player
            .render(
                frames,
                transport,
                Navigation::SYNTH_RACK.assignments,
                enabled,
                |track, event| {
                    events.push((track, event));
                    Ok(())
                },
            )
            .expect("unbounded host event sink");
        events
    }

    #[test]
    fn panel_browses_four_tracks_and_edits_one_global_bpm() {
        let mut seq = Sequencer::INITIAL;
        assert_eq!(seq.enabled, [true; 4]);
        seq.channel.turn(-1);
        assert_eq!(seq.channel, Channel::Four);
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
        assert!(seq.back());
        assert!(!seq.back());
        assert_eq!(seq.page, Some(SeqPage::Pattern));
        seq.toggle_page();
        assert_eq!(seq.page, None);
        assert!(seq.transport.playing());
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
        fn collect(blocks: &[usize]) -> Vec<(Channel, Event<Message>)> {
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
    fn factory_808_fills_the_909_rests_and_both_303s_play_their_own_slides() {
        let mut counts = [0; 4];
        for i in 0..pattern::STEPS {
            for voice in pattern::drums(Part::Tr808, i) {
                assert!(pattern::drums(Part::Tr909, i).is_empty());
                let index = Part::Tr808
                    .pads()
                    .iter()
                    .position(|v| v == voice)
                    .expect("factory hit has an 808 pad");
                counts[index] += 1;
            }
        }
        assert_eq!(counts, [4, 4, 6, 3]);

        let mut player = PatternPlayer::new((48_000, 1));
        let mut slides = Vec::new();
        player
            .render(
                48_000,
                Transport {
                    bpm: 120,
                    ..playing()
                },
                Navigation::INITIAL.assignments,
                [true; CHANNELS],
                |channel, event| {
                    if let Message::Bass(BassMessage::NoteOn {
                        note, slide: true, ..
                    }) = event.message
                    {
                        slides.push((channel, event.frame, note));
                    }
                    Ok(())
                },
            )
            .expect("unbounded host event sink");
        assert_eq!(
            slides,
            [(Channel::Two, 36_000, 65), (Channel::One, 42_000, 48)]
        );
    }

    #[test]
    fn factory_drums_repeat_only_the_four_house_voices() {
        use wavegraph_909::Voice::{Clap, ClosedHat, Kick, Snare};
        let mut player = PatternPlayer::new((48_000, 1));
        let events = render(
            &mut player,
            384_000,
            Transport {
                bpm: 120,
                ..playing()
            },
            [false, false, false, true],
        );
        let hits: Vec<_> = events
            .into_iter()
            .filter_map(|(_, e)| match e.message {
                Message::Drum(DrumMessage::Trigger(v, _)) => Some((e.frame, v)),
                _ => None,
            })
            .collect();
        let mut expected = Vec::new();
        for beat in 0..16 {
            let start = beat * 24_000;
            expected.push((start, Kick));
            if beat % 2 == 1 {
                expected.extend([(start, Snare), (start, Clap)]);
            }
            expected.push((start + 12_000, ClosedHat));
        }
        assert_eq!(hits, expected);
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
        let mut previous = pattern::automation_values(Channel::Three, 0, 0);
        let mut changes = [false; 3];
        for step in 0..pattern::STEPS {
            for fraction in 0..=255 {
                let values = pattern::automation_values(Channel::Three, step, fraction);
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
        let first = pattern::automation_values(Channel::Three, 0, 0);
        for lane in 0..3 {
            assert!((previous[lane] - first[lane]).abs() < 1.0);
        }
    }

    #[test]
    fn a_full_event_sink_is_an_explicit_failure() {
        let mut player = PatternPlayer::new((44_100, 1));
        assert_eq!(
            player.render(
                128,
                playing(),
                Navigation::SYNTH_RACK.assignments,
                [true; 4],
                |_, _| Err(Error::Capacity)
            ),
            Err(Error::Capacity)
        );
    }

    #[test]
    fn drum_trigger_and_stop_obey_the_shared_frame_offsets() {
        let mut drums = DrumKits::new((44_100, 1)).assign(Part::Tr909);
        let mut out = [I24::default(); 256];
        let mut kick = out;
        drums.process(&mut ProcessContext {
            frames: 256,
            inputs: &[],
            outputs: &mut [&mut out, &mut kick],
            events: &[
                event(11, Message::Drum(DrumMessage::Trigger(Voice::Kick, 127))),
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
