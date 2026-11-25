//! FM-1 master transport and one audio-clocked player for all factory patterns.

use crate::bass::{self, CC_SLIDE};
use crate::{
    instrument::{Message, Part, Track},
    panel::{Assignment, CHANNELS, Channel, Navigation},
};
use pattern::{Automation, BAR, ChannelSlots, Clip, Slot};
use wavegraph_909::{Tr909, Voice};
use wavegraph_dsp::{Event, Port, Process, ProcessContext, bytecode::Error, midi::MidiMessage};
use wavegraph_sample::I24;

pub(super) mod edit;
pub(super) mod pattern;
use edit::{Address, EDITS, Edits, Step, Suppressed};

const SUBSTEPS: usize = pattern::STEPS * 8;
// Local clip modulation of the first channel insert's Saturator Shape.
const CC_SATURATOR_SHAPE: u8 = 20;

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
    Record,
    Step,
}

/// The stock sequencer puts tempo on page two, knob three.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Sequencer {
    pub(super) page: Option<SeqPage>,
    pub(super) transport: Transport,
    pub(super) channel: Channel,
    pub(super) bar: u8,
    pub(super) cursor: u8,
    pub(super) edit_revision: u32,
    pub(super) full: bool,
    pub(super) enabled: [bool; CHANNELS],
    /// The pressed bank key is in the dark half of its blink.
    pub(super) pressed_key_dark: bool,
    selected: [ChannelSlots; Part::ALL.len()],
}

impl Sequencer {
    pub(super) const INITIAL: Self = Self {
        page: None,
        transport: Transport::INITIAL,
        channel: Channel::One,
        bar: 0,
        cursor: 0,
        edit_revision: 0,
        full: false,
        enabled: [true; CHANNELS],
        pressed_key_dark: false,
        selected: [ChannelSlots::FIRST; Part::ALL.len()],
    };

    pub(super) fn slot(&self, channel: Channel, part: Part) -> Slot {
        self.selected[part as usize].get(channel)
    }

    pub(super) fn choose(&mut self, part: Part, slot: Slot) {
        self.selected[part as usize].set(self.channel, slot);
        self.bar = 0;
    }

    pub(super) fn toggle_page(&mut self) {
        self.page = if self.page.is_none() {
            Some(SeqPage::Pattern)
        } else {
            None
        };
    }

    pub(super) fn back(&mut self) -> bool {
        if matches!(
            self.page,
            Some(SeqPage::Tempo | SeqPage::Record | SeqPage::Step)
        ) {
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
    pub(super) fn turn_bar(&mut self, part: Part, detents: i8) {
        if !self.transport.playing() || self.page == Some(SeqPage::Step) {
            let bars = self.slot(self.channel, part).clip(part, self.channel).bars;
            self.bar = u8::try_from(
                (i16::from(self.bar) + i16::from(detents)).rem_euclid(i16::from(bars)),
            )
            .expect("four bars");
        }
    }

    pub(super) fn record(&mut self) {
        if self.page == Some(SeqPage::Record) {
            self.page = Some(SeqPage::Pattern);
        } else {
            if !self.transport.playing() {
                self.transport.toggle();
            }
            self.page = Some(SeqPage::Record);
        }
        self.full = false;
    }

    pub(super) fn step_edit(&mut self) {
        self.page = if self.page == Some(SeqPage::Step) {
            Some(SeqPage::Pattern)
        } else {
            Some(SeqPage::Step)
        };
        self.full = false;
    }

    pub(super) fn turn_cursor(&mut self, delta: i8) {
        self.cursor = self.cursor.wrapping_add_signed(delta) & 15;
    }

    pub(super) fn edit_address(
        &self,
        part: Part,
        playing: Option<(Position, BankPosition)>,
    ) -> Option<Address> {
        let selected = self.slot(self.channel, part);
        let (slot, bar, step) = match self.page? {
            SeqPage::Record => playing.map_or((selected, 0, 0), |(pos, bank)| {
                (bank.slot, bank.bar, pos.step % 16)
            }),
            SeqPage::Step => (
                selected,
                self.bar
                    .checked_rem(selected.clip(part, self.channel).bars)?,
                self.cursor,
            ),
            _ => return None,
        };
        Some(Address {
            channel: self.channel,
            part,
            slot,
            bar,
            step,
        })
    }

    pub(super) fn toggle_track(&mut self) {
        let enabled = &mut self.enabled[self.channel.index()];
        *enabled = !*enabled;
    }

    pub(super) fn turn(&mut self, select: i8, knob3: i8) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if matches!(page, SeqPage::Record | SeqPage::Step) {
            return;
        }
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
        // The pattern repeats every 16 beats. A three-beat FX sweep also
        // needs its phase beyond that boundary; both grids repeat after 48.
        self.next = (tick + 1) % (3 * SUBSTEPS);
        Some(tick % SUBSTEPS)
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

    /// Loop position at the last rendered frame.
    /// Quarter-note phase shares the same sub-frame remainder as drum triggers.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "Normalized tick positions and their fractions stay within 0..256."
    )]
    fn position(&self) -> Position {
        let tick = self.next.wrapping_sub(1) % SUBSTEPS;
        let fine = fraction_32(self.phase - self.increment, self.threshold);
        let beat_phase = (tick % 32) * 8 + usize::from(fine >> 2);
        Position {
            step: (tick / 8) as u8,
            beat_phase: beat_phase as u8,
        }
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BankPosition {
    pub(crate) slot: Slot,
    pub(crate) bar: u8,
    pub(crate) house_m7: bool,
}

#[derive(Clone, Copy)]
struct Lane {
    position: BankPosition,
    tick: u16,
    cursor: usize,
    held: [Option<u8>; 4],
    automation: Automation,
    slide: bool,
    waiting: bool,
    edited: Step,
    suppressed: Suppressed,
}

impl Lane {
    const EMPTY: Self = Self {
        position: BankPosition {
            slot: Slot::FIRST,
            bar: 0,
            house_m7: false,
        },
        tick: 0,
        cursor: 0,
        held: [None; 4],
        automation: Automation::EMPTY,
        slide: false,
        waiting: false,
        edited: Step::EMPTY,
        suppressed: Suppressed::EMPTY,
    };

    fn start(slot: Slot, house_m7: bool) -> Self {
        Self {
            position: BankPosition {
                slot,
                bar: 0,
                house_m7,
            },
            ..Self::EMPTY
        }
    }

    fn release(&mut self, route: &mut Route<'_>, cut_tails: bool) -> Result<(), Error> {
        let part = route.track.assignment.part;
        self.release_notes(route)?;
        if cut_tails && part.is_kit() {
            route.graph(Message::Drum(DrumMessage::Stop))?;
        } else if cut_tails && part == Part::Sampler {
            route.graph(Message::Sampler(wavegraph_sampler::PitchedMessage::Stop))?;
        }
        if self.slide {
            route.wire(MidiMessage::Control(CC_SLIDE, 0));
        }
        Ok(())
    }

    fn release_notes(&mut self, route: &mut Route<'_>) -> Result<(), Error> {
        let part = route.track.assignment.part;
        for held in &mut self.held {
            if let Some(n) = held.take() {
                route.wire(MidiMessage::Off(n));
                route.graph(part.note_off(n))?;
            }
        }
        Ok(())
    }

    fn restore(&self, route: &mut Route<'_>) -> Result<(), Error> {
        for (cc, _) in self.automation.values() {
            if let Some(value) = route.track.bass.midi_value(cc, route.track.voice) {
                route.wire(MidiMessage::Control(cc, value));
            }
            if let Some(message) = route.track.bass.restore(cc, route.track.voice) {
                route.graph(Message::Bass(message))?;
            }
        }
        Ok(())
    }

    fn chase(&self, route: &mut Route<'_>) -> Result<(), Error> {
        for (cc, value) in self.automation.values() {
            route.wire(MidiMessage::Control(cc, value));
            if let Some(message) = bass::midi_parameter(cc, value) {
                route.graph(Message::Bass(message))?;
            }
        }
        if route.track.assignment.part == Part::Bass {
            route.wire(MidiMessage::Control(
                CC_SLIDE,
                if self.slide { 127 } else { 0 },
            ));
        }
        Ok(())
    }

    fn dispatch(
        &mut self,
        midi: MidiMessage,
        enabled: bool,
        route: &mut Route<'_>,
    ) -> Result<(), Error> {
        let part = route.track.assignment.part;
        if let MidiMessage::Control(cc, value) = midi
            && part == Part::Bass
        {
            if cc == CC_SATURATOR_SHAPE {
                // A zero offset gives the knob its full range. Keep the local
                // FX lane off the USB stream sent to an external TB-03.
                self.automation.shape_offset = value.saturating_add(value >> 6);
                return Ok(());
            }
            if cc == CC_SLIDE {
                self.slide = value >= 64;
            } else {
                self.automation.set(cc, value);
            }
        }
        if !enabled {
            return Ok(());
        }
        route.wire(midi);
        let message = match midi {
            MidiMessage::On(n) if part.is_kit() => part
                .drum(n.number)
                .map(|v| Message::Drum(DrumMessage::Trigger(v, n.velocity))),
            MidiMessage::Off(_) if part.is_kit() => None,
            MidiMessage::On(n) => {
                let legato = self.slide || self.held.iter().any(Option::is_some);
                if part.is_polyphonic() {
                    if !self.held.contains(&Some(n.number)) {
                        let slot = self
                            .held
                            .iter_mut()
                            .find(|slot| slot.is_none())
                            .ok_or(Error::Capacity)?;
                        *slot = Some(n.number);
                    }
                } else {
                    self.held[0] = Some(n.number);
                }
                Some(part.note_on(n, legato))
            }
            MidiMessage::Off(n) => self.held.iter_mut().find(|v| **v == Some(n)).map(|slot| {
                *slot = None;
                part.note_off(n)
            }),
            MidiMessage::Control(cc, v) if part == Part::Bass => {
                bass::midi_parameter(cc, v).map(Message::Bass)
            }
            MidiMessage::Control(..) => None,
        };
        if let Some(message) = message {
            route.graph(message)?;
        }
        Ok(())
    }

    fn read(&mut self, clip: &Clip, enabled: bool, route: &mut Route<'_>) -> Result<(), Error> {
        while let Some(e) = clip.events.get(self.cursor) {
            if e.tick() != self.tick {
                break;
            }
            let msg = e.message();
            if self
                .suppressed
                .factory(self.edited, route.track.assignment.part, msg, self.tick % 8)
            {
                self.dispatch(msg, enabled, route)?;
            }
            self.cursor = self.cursor.saturating_add(1);
        }
        Ok(())
    }
}

/// One route preserves the MIDI stream for USB and translates it at the assigned graph inlet.
struct Route<'a> {
    frame: usize,
    channel: Channel,
    track: &'a Track,
    graph: &'a mut dyn FnMut(Channel, Event<Message>) -> Result<(), Error>,
    wire: &'a mut dyn FnMut(Channel, Event<MidiMessage>),
}
impl Route<'_> {
    fn graph(&mut self, m: Message) -> Result<(), Error> {
        (self.graph)(self.channel, event(self.frame, m))
    }
    fn wire(&mut self, m: MidiMessage) {
        (self.wire)(
            self.channel,
            Event {
                frame: self.frame,
                port: 0,
                message: m,
            },
        );
    }
}

/// One audio clock for all four independent clip cursors.
#[derive(Clone, Copy)]
pub(super) struct PatternPlayer {
    edits: &'static Edits,
    clock: StepClock,
    transport: Transport,
    assignments: [Assignment; CHANNELS],
    enabled: [bool; CHANNELS],
    lanes: [Lane; CHANNELS],
    pub(super) requested: [Slot; CHANNELS],
    position: Option<Position>,
    pub(super) downbeat: Option<usize>,
}

impl PatternPlayer {
    pub(super) fn transport_changed(&self, transport: Transport) -> bool {
        self.transport.generation != transport.generation
    }

    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            edits: &EDITS,
            clock: StepClock::new(rate.0, rate.1),
            transport: Transport::INITIAL,
            assignments: Navigation::INITIAL.assignments,
            enabled: [false; CHANNELS],
            lanes: [Lane::EMPTY; CHANNELS],
            requested: [Slot::FIRST; CHANNELS],
            position: None,
            downbeat: None,
        }
    }

    pub(super) fn position(&self) -> Option<Position> {
        self.position
    }

    /// Phase at the first frame of the upcoming block, before the player advances.
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "32 substeps per beat; the scaled division is bounded by one beat plus one sample"
    )]
    pub(super) fn effect_phase(
        &self,
        transport: Transport,
        clock: wavegraph_fx::BeatClock,
    ) -> Option<u32> {
        if !transport.playing() {
            return None;
        }
        if self.transport_changed(transport) {
            return Some(0);
        }
        let c = &self.clock;
        let tick = (c.next + 3 * SUBSTEPS - 1) % (3 * SUBSTEPS);
        let fine = (u64::from(c.phase) << 27) / u64::from(c.threshold);
        let fraction = ((tick as u64 % 32) << 27) + fine;
        Some(clock.phase(
            (tick / 32) as u32 + (fraction >> 32) as u32,
            fraction as u32,
        ))
    }

    pub(super) fn banks(&self) -> [BankPosition; CHANNELS] {
        self.lanes.map(|l| l.position)
    }

    pub(super) fn automation(&self, channel: Channel) -> Option<Automation> {
        let i = channel.index();
        (self.transport.playing() && self.enabled[i] && self.assignments[i].part == Part::Bass)
            .then_some(self.lanes[i].automation)
    }

    pub(super) fn render(
        &mut self,
        frames: usize,
        transport: Transport,
        tracks: &[Track; CHANNELS],
        enabled: [bool; CHANNELS],
        emit: impl FnMut(Channel, Event<Message>) -> Result<(), Error>,
        wire: impl FnMut(Channel, Event<MidiMessage>),
    ) -> Result<(), Error> {
        self.render_steps(
            frames,
            transport,
            tracks,
            enabled,
            emit,
            (wire, |_, _| Ok(())),
        )
    }

    pub(super) fn render_steps(
        &mut self,
        frames: usize,
        transport: Transport,
        tracks: &[Track; CHANNELS],
        enabled: [bool; CHANNELS],
        mut emit: impl FnMut(Channel, Event<Message>) -> Result<(), Error>,
        (mut wire, mut step): (
            impl FnMut(Channel, Event<MidiMessage>),
            impl FnMut(usize, u8) -> Result<(), Error>,
        ),
    ) -> Result<(), Error> {
        self.downbeat = None;
        if frames == 0 {
            return Ok(());
        }
        let restart = self.transport_changed(transport);
        for channel in Channel::ALL {
            let i = channel.index();
            let mut route = Route {
                frame: 0,
                channel,
                track: &tracks[i],
                graph: &mut emit,
                wire: &mut wire,
            };
            let changed = self.assignments[i] != tracks[i].assignment;
            let lane = &mut self.lanes[i];
            if self.transport.playing() && self.enabled[i] && (restart || changed || !enabled[i]) {
                route.wire(MidiMessage::Control(123, 0));
                if changed {
                    route.wire(MidiMessage::Control(CC_SLIDE, 0));
                } else {
                    lane.release(&mut route, true)?;
                    lane.restore(&mut route)?;
                }
            }
            if restart || changed || !transport.playing() {
                *lane = Lane::start(
                    self.requested[i],
                    tracks[i]
                        .assignment
                        .part
                        .uses_house_patterns(tracks[i].voice.waveform),
                );
                lane.waiting = changed && !restart && transport.playing();
            } else if enabled[i] && !self.enabled[i] {
                lane.chase(&mut route)?;
            }
            self.assignments[i] = tracks[i].assignment;
        }
        if restart {
            self.clock.restart();
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
                if tick.is_multiple_of(8) {
                    step(
                        frame,
                        u8::try_from(tick / 8 % 16).map_err(|_| Error::Capacity)?,
                    )?;
                }
                if tick.is_multiple_of(16 * 8) && self.downbeat.is_none() {
                    self.downbeat = Some(frame);
                }
                for channel in Channel::ALL {
                    let i = channel.index();
                    let lane = &mut self.lanes[i];
                    let part = tracks[i].assignment.part;
                    let mut route = Route {
                        frame,
                        channel,
                        track: &tracks[i],
                        graph: &mut emit,
                        wire: &mut wire,
                    };
                    let downbeat = tick.is_multiple_of(usize::from(BAR));
                    if lane.waiting && !downbeat {
                        continue;
                    }
                    let old =
                        lane.position
                            .slot
                            .factory_clip(part, channel, lane.position.house_m7);
                    let wrap = lane.tick == old.length();
                    let house_m7 = part.uses_house_patterns(tracks[i].voice.waveform);
                    let switch = downbeat
                        && (lane.position.slot != self.requested[i]
                            || lane.position.house_m7 != house_m7);
                    if wrap || switch || lane.waiting {
                        if wrap {
                            lane.edited = Step::EMPTY;
                            lane.read(old, enabled[i], &mut route)?;
                        }
                        if enabled[i] && !lane.waiting {
                            lane.release(&mut route, false)?;
                            if switch {
                                route.wire(MidiMessage::Control(123, 0));
                                lane.restore(&mut route)?;
                            }
                        }
                        let slot = if switch {
                            self.requested[i]
                        } else {
                            lane.position.slot
                        };
                        // Controllers latch across repeats of the same MIDI clip.
                        let controls = if wrap && !switch {
                            lane.automation
                        } else {
                            Automation::EMPTY
                        };
                        *lane = Lane::start(slot, house_m7);
                        lane.automation = controls;
                    }
                    let clip =
                        lane.position
                            .slot
                            .factory_clip(part, channel, lane.position.house_m7);
                    lane.position.bar =
                        u8::try_from(lane.tick / BAR).map_err(|_| Error::Capacity)?;
                    let phase = lane.tick % 8;
                    if phase == 0 {
                        lane.edited = self.edits.step(Address {
                            channel,
                            part,
                            slot: lane.position.slot,
                            bar: lane.position.bar,
                            step: u8::try_from(lane.tick % BAR / 8).map_err(|_| Error::Capacity)?,
                        });
                        if enabled[i] && lane.edited.edited() && !part.is_kit() {
                            for &n in lane.held.iter().flatten() {
                                lane.suppressed.cut(n);
                            }
                            lane.release_notes(&mut route)?;
                        }
                    }
                    lane.read(clip, enabled[i], &mut route)?;
                    let step = lane.edited;
                    for msg in step.events(phase) {
                        lane.dispatch(msg, enabled[i], &mut route)?;
                    }
                    lane.tick = lane.tick.saturating_add(1);
                }
            }
            frame = frame.saturating_add(self.clock.advance(frames.saturating_sub(frame)));
        }
        self.position = Some(self.clock.position());
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
    Decay909([u8; 3]),
    Stop,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DrumLamp {
    #[default]
    Off,
    Tail,
    Strike,
}

pub(super) const DECAY_909: u8 = 55;

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
            tr909: crate::assets::tr909(rate, DECAY_909).expect("909 clock and PCM"),
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
            (Self::Tr909(kit), DrumMessage::Decay909(decay)) => kit.set_decay(decay),
            (Self::Tr909(_), DrumMessage::Controls808(_))
            | (Self::Tr808(_), DrumMessage::Decay909(_)) => (),
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
    use wavegraph_303::Message as BassMessage;
    extern crate alloc;

    use super::*;
    use alloc::vec::Vec;

    fn playing() -> Transport {
        let mut transport = Transport::INITIAL;
        transport.toggle();
        transport
    }

    #[test]
    fn sampler_pattern_one_retriggers_the_break_on_each_four_bar_downbeat() {
        use wavegraph_sampler::PitchedMessage;

        let channel = Channel::Three;
        let mut tracks = Track::rack(Navigation::INITIAL.assignments);
        let track = &mut tracks[channel.index()];
        assert_eq!(track.assignment.part, Part::Tr808);
        *track = Track::factory(channel, Assignment::new(Part::Sampler));
        assert_eq!(track.voice.label(Part::Sampler), b"AMEN BRK ");
        let mut player = PatternPlayer::new((24_000_000, 544));
        let mut events = Vec::new();
        for start in (0..700_000).step_by(128) {
            player
                .render(
                    128,
                    playing(),
                    &tracks,
                    [false, false, true, false],
                    |ch, event| {
                        if ch == channel {
                            match event.message {
                                Message::Sampler(PitchedMessage::NoteOn(note)) => {
                                    events.push((start + event.frame, note.number, true))
                                }
                                Message::Sampler(PitchedMessage::NoteOff(note)) => {
                                    events.push((start + event.frame, note, false))
                                }
                                _ => (),
                            }
                        }
                        Ok(())
                    },
                    |_, _| (),
                )
                .expect("host event sink");
        }
        // ceil(n * 16 beats * 60 s/min * (24 MHz / 544) / 122 BPM).
        assert_eq!(
            events,
            [
                (0, 60, true),
                (347_156, 60, false),
                (347_156, 60, true),
                (694_311, 60, false),
                (694_311, 60, true)
            ]
        );
    }

    #[test]
    fn house_patterns_keep_the_whole_sampled_chord_in_d_minor() {
        let part = Part::Sampler;
        let channel = Channel::Three;
        for (_, slot) in Slot::KEYS {
            let clip = slot.factory_clip(part, channel, true);
            assert_eq!(clip.bars, slot.clip(part, channel).bars);
            for root in clip.pitches {
                for interval in [0, 3, 7, 10] {
                    assert!([2, 4, 5, 7, 9, 10, 0].contains(&((root + interval) % 12)));
                }
            }
        }
        let clip = Slot::FIRST.factory_clip(part, channel, true);
        assert!(matches!(clip.events[0].message(), MidiMessage::On(n) if n.number == 62));
        assert!(
            clip.events
                .windows(2)
                .any(|pair| pair[0].tick() == pair[1].tick()
                    && matches!(
                        (pair[0].message(), pair[1].message()),
                        (MidiMessage::On(_), MidiMessage::On(_))
                    ))
        );
    }

    #[test]
    fn selecting_the_stab_releases_the_break_and_starts_on_d_at_the_next_bar() {
        let channel = Channel::Three;
        let mut tracks = Track::rack(Navigation::INITIAL.assignments);
        tracks[channel.index()] = Track::factory(channel, Assignment::new(Part::Sampler));
        let mut player = PatternPlayer::new((3200, 1));
        let transport = Transport {
            bpm: 150,
            ..playing()
        }; // 5120 frames per bar
        let mut notes = Vec::new();
        for (offset, frames, sample) in [(0, 1, 0), (1, 5120, crate::assets::HOUSE_M7)] {
            tracks[channel.index()].voice.waveform = sample;
            player
                .render(
                    frames,
                    transport,
                    &tracks,
                    [false, false, true, false],
                    |_, _| Ok(()),
                    |ch, event| {
                        if ch == channel {
                            match event.message {
                                MidiMessage::On(n) => {
                                    notes.push((offset + event.frame, n.number, true))
                                }
                                MidiMessage::Off(n) => notes.push((offset + event.frame, n, false)),
                                _ => (),
                            }
                        }
                    },
                )
                .expect("host event sink");
        }
        assert_eq!(notes, [(0, 60, true), (5120, 60, false), (5120, 62, true)]);
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
                &Track::rack(Navigation::SYNTH_RACK.assignments),
                enabled,
                |track, event| {
                    events.push((track, event));
                    Ok(())
                },
                |_, _| (),
            )
            .expect("unbounded host event sink");
        events
    }

    #[test]
    fn rec_targets_the_sounding_bank_and_step_mode_targets_the_cursor() {
        let mut seq = Sequencer::INITIAL;
        let next = Slot::KEYS[2].1;
        seq.choose(Part::Bass, next);
        seq.record();
        assert!(seq.transport.playing());
        let sounding = (
            Position {
                step: 51,
                beat_phase: 0,
            },
            BankPosition {
                house_m7: false,
                slot: Slot::FIRST,
                bar: 1,
            },
        );
        let address = |slot, bar, step| Address {
            channel: Channel::One,
            part: Part::Bass,
            slot,
            bar,
            step,
        };
        assert_eq!(
            seq.edit_address(Part::Bass, Some(sounding)),
            Some(address(Slot::FIRST, 1, 3))
        );
        assert_eq!(
            seq.edit_address(Part::Bass, None),
            Some(address(next, 0, 0))
        );
        seq.record();
        assert!(seq.transport.playing());
        assert_eq!(seq.edit_address(Part::Bass, Some(sounding)), None);
        seq.step_edit();
        seq.turn_cursor(-1);
        assert_eq!(
            seq.edit_address(Part::Bass, Some(sounding)),
            Some(address(next, 0, 15))
        );
        seq.turn_cursor(1);
        assert_eq!(seq.cursor, 0);
        assert!(seq.back());
        assert_eq!(seq.page, Some(SeqPage::Pattern));
    }

    #[test]
    fn live_909_burst_fits_an_edited_step_with_alternate_drum_pitches() -> Result<(), Error> {
        use crate::instrument::RoutedEvents;
        use wavegraph_dsp::midi::Note;

        let channel = Channel::Four;
        let track = &mut Track::rack(Navigation::INITIAL.assignments)[channel.index()];
        let edits = Edits::new();
        let at = Address {
            channel,
            part: Part::Tr909,
            slot: Slot::FIRST,
            bar: 0,
            step: 0,
        };
        let note = |number| Note {
            number,
            velocity: 100,
        };
        for pitch in [39, 46, 51] {
            assert!(edits.write(at, note(pitch), false));
        }
        let step = edits.step(at);
        let mut events = RoutedEvents::new();
        for _ in 0..8 {
            track
                .drum_hits_mut()
                .expect("kit")
                .panel
                .push(Voice::Kick, 82);
        }
        for _ in 0..32 {
            track
                .drum_hits_mut()
                .expect("kit")
                .midi
                .push(Voice::Snare, 88);
        }
        track
            .drum_hits_mut()
            .expect("kit")
            .panel
            .append_since(0, &mut events)?;
        track
            .drum_hits_mut()
            .expect("kit")
            .midi
            .append_since(0, &mut events)?;
        let mut lane = Lane::EMPTY;
        let mut route = Route {
            frame: 0,
            channel,
            track,
            graph: &mut |_, event| events.push(event).map_err(|_| Error::Capacity),
            wire: &mut |_, _| {},
        };
        // Six simultaneous imported pitches use three 909 voices. The panel
        // edits on the other three voices leave every imported strike audible.
        for pitch in [35, 36, 38, 40, 42, 44] {
            let msg = MidiMessage::On(note(pitch));
            if lane.suppressed.factory(step, Part::Tr909, msg, 0) {
                lane.dispatch(msg, true, &mut route)?;
            }
        }
        for msg in step.events(0) {
            lane.dispatch(msg, true, &mut route)?;
        }
        assert_eq!(events.len(), 49);
        Ok(())
    }

    #[test]
    fn edited_notes_loop_with_velocity_and_releases_while_other_lanes_keep_playing() {
        use alloc::boxed::Box;
        use wavegraph_dsp::midi::Note;
        let edits = Box::leak(Box::new(Edits::new()));
        let slot = Slot::KEYS[2].1; // a single bar
        let addr = |channel, part| Address {
            channel,
            part,
            slot,
            bar: 0,
            step: 0,
        };
        let note = |number, velocity| Note { number, velocity };
        let bass = addr(Channel::One, Part::Bass);
        let kit = addr(Channel::Four, Part::Tr909);
        assert!(edits.write(bass, note(91, 79), false));
        assert!(edits.write(kit, note(51, 88), false));
        assert!(edits.write(kit, note(36, 90), true));
        let mut player = PatternPlayer::new((3200, 1));
        player.requested = [slot; 4];
        player.edits = edits;
        let mut transport = playing();
        transport.bpm = 150; // 40 frames/tick, 5120 frames/bar
        let mut wire = Vec::new();
        let mut graph = Vec::new();
        let tracks = Track::rack(Navigation::INITIAL.assignments);
        let mut block = |player: &mut PatternPlayer, frames, start| {
            player
                .render(
                    frames,
                    transport,
                    &tracks,
                    [true; 4],
                    |c, mut e| {
                        e.frame += start;
                        graph.push((c, e));
                        Ok(())
                    },
                    |c, mut e| {
                        e.frame += start;
                        wire.push((c, e));
                    },
                )
                .expect("host sink")
        };
        block(&mut player, 1, 0);
        assert!(edits.write(bass, note(92, 114), false));
        block(&mut player, 5200, 1);
        for (frame, message) in [
            (0, MidiMessage::On(note(91, 79))),
            (240, MidiMessage::Off(91)),
            (5120, MidiMessage::On(note(92, 114))),
        ] {
            assert!(
                wire.iter()
                    .any(|(c, e)| *c == Channel::One && e.frame == frame && e.message == message)
            );
        }
        assert!(graph.iter().any(|(c, e)| *c == Channel::Four
            && e.frame == 5120
            && e.message == Message::Drum(DrumMessage::Trigger(Voice::Ride, 88))));
        assert!(!graph.iter().any(|(c, e)| *c == Channel::Four
            && e.frame == 0
            && matches!(
                e.message,
                Message::Drum(DrumMessage::Trigger(Voice::Kick, _))
            )));
        let factory = slot
            .clip(Part::Tr909, Channel::Four)
            .events
            .iter()
            .filter(|e| e.tick() == 0);
        for n in factory.filter_map(|e| match e.message() {
            MidiMessage::On(n) if !matches!(n.number, 35 | 36 | 51) => Some(n),
            _ => None,
        }) {
            assert!(wire.iter().any(|(c, e)| *c == Channel::Four
                && e.frame == 0
                && e.message == MidiMessage::On(n)));
        }
        assert!(
            wire.iter()
                .any(|(c, e)| *c == Channel::Two && matches!(e.message, MidiMessage::On(_)))
        );
    }

    #[test]
    fn panel_browses_four_tracks_and_edits_one_global_bpm() {
        let mut seq = Sequencer::INITIAL;
        assert_eq!(seq.enabled, [true; 4]);
        seq.channel.turn(-1);
        assert_eq!(seq.channel, Channel::Four);
        seq.toggle_track();
        assert!(!seq.enabled[3]);
        seq.turn_bar(Part::Bass, -1);
        assert_eq!(seq.bar, 3);
        seq.turn(1, 127);
        assert_eq!(seq.transport.bpm, 122);
        seq.toggle_page();
        seq.turn(0, 127);
        assert_eq!(seq.transport.bpm, 122);
        seq.turn(1, 2);
        assert_eq!(seq.transport.bpm, 124);
        seq.transport.toggle();
        seq.turn_bar(Part::Bass, 1);
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
            assert_eq!(clock.position().beat_phase, expected);
            if tick.is_some_and(|tick| tick % 32 == 0) {
                assert_eq!(clock.position().beat_phase, 0);
            }
            beat_units = (beat_units + if frame < 9000 { 2 } else { 3 }) % 48_000;
        }
    }

    #[test]
    fn three_beat_sweep_crosses_pattern_wrap_and_restarts_with_transport() {
        use wavegraph_fx::{BeatClock, BeatDivision};
        let mut transport = Transport {
            bpm: 120,
            ..playing()
        };
        let mut clock = BeatClock {
            beats: BeatDivision::from_ratio(3, 1).expect("three beats"),
            bpm: transport.bpm,
        };
        let mut player = PatternPlayer::new((8000, 1));
        assert_eq!(player.effect_phase(transport, clock), Some(0));
        render(&mut player, 16 * 4000, transport, [false; 4]);
        assert_eq!(
            player.effect_phase(transport, clock),
            Some(clock.phase(16, 0))
        );
        transport.bpm = 60;
        clock.bpm = transport.bpm;
        render(&mut player, 4000, transport, [false; 4]);
        assert_eq!(
            player.effect_phase(transport, clock),
            Some(clock.phase(16, 1 << 31))
        );
        transport.toggle();
        assert_eq!(player.effect_phase(transport, clock), None);
        transport.toggle();
        assert_eq!(player.effect_phase(transport, clock), Some(0));
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
        let kit_steps = |part| {
            let mut steps = [0_u8; 64];
            for &e in Slot::FIRST.clip(part, Channel::Three).events {
                if let MidiMessage::On(n) = e.message() {
                    let drum = part.drum(n.number).expect("supported kit voice");
                    steps[usize::from(e.tick() / 8)] |= 1 << drum as usize;
                }
            }
            steps
        };
        let fills = kit_steps(Part::Tr808);
        let main = kit_steps(Part::Tr909);
        for (a, b) in fills.iter().zip(main) {
            assert!(*a == 0 || b == 0);
        }
        assert_eq!(
            Part::Tr808
                .pads()
                .iter()
                .map(|v| fills
                    .iter()
                    .filter(|mask| **mask & (1 << *v as usize) != 0)
                    .count())
                .collect::<Vec<_>>(),
            [4, 4, 6, 3]
        );

        let mut player = PatternPlayer::new((48_000, 1));
        let mut slides = Vec::new();
        player
            .render(
                48_000,
                Transport {
                    bpm: 120,
                    ..playing()
                },
                &Track::rack(Navigation::INITIAL.assignments),
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
                |_, _| (),
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
        assert_eq!(player.automation(Channel::Three), None);
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
        assert_eq!(player.automation(Channel::Three), None);
        assert!(
            stopped
                .iter()
                .any(|(_, event)| event.message == Message::Bass(BassMessage::NoteOff(38)))
        );
        assert!(render(&mut player, 128, restart, [true; 4]).is_empty());
    }

    #[test]
    fn bass_lane_chases_external_delay_controls_after_skipped_frames() -> Result<(), Error> {
        let tracks = Track::rack(Navigation::INITIAL.assignments);
        let mut lane = Lane::EMPTY;
        let mut wire = Vec::new();
        let mut route = Route {
            frame: 0,
            channel: Channel::One,
            track: &tracks[Channel::One.index()],
            graph: &mut |_, _| unreachable!("external delay has no graph parameter"),
            wire: &mut |_, event| wire.push(event.message),
        };
        // A clip can change the same knob several times while the lane is parked.
        for (cc, value) in [(18, 10), (19, 20), (18, 42)] {
            lane.dispatch(MidiMessage::Control(cc, value), false, &mut route)?;
        }
        lane.chase(&mut route)?;
        assert_eq!(
            wire,
            [
                MidiMessage::Control(18, 42),
                MidiMessage::Control(19, 20),
                MidiMessage::Control(CC_SLIDE, 0)
            ]
        );
        Ok(())
    }

    #[test]
    fn original_acid_filter_and_drive_follow_the_four_bar_sweep() {
        let clip = Slot::FIRST.clip(Part::Bass, Channel::One);
        let mut controls = Automation::EMPTY;
        let mut cursor = 0;
        for (segment, (hz, res, drive)) in [
            (180., 0.76, 12.),
            (380., 0.88, 18.),
            (1200., 0.94, 27.),
            (300., 0.80, 15.),
            (220., 0.74, 14.),
            (620., 0.89, 22.),
            (1800., 0.93, 28.),
            (540., 0.84, 20.),
        ]
        .into_iter()
        .enumerate()
        {
            while let Some(e) = clip.events.get(cursor)
                && usize::from(e.tick()) <= segment * 64
            {
                if let MidiMessage::Control(cc, value) = e.message() {
                    controls.set(cc, value);
                }
                cursor += 1;
            }
            let parameter =
                |cc| bass::midi_parameter(cc, controls.get(cc).expect("automated lane"));
            assert!(
                matches!(parameter(74), Some(BassMessage::CutoffHz(v)) if (v-hz).abs() <= hz * 0.021)
            );
            assert!(
                matches!(parameter(71), Some(BassMessage::Resonance(v)) if (v-res).abs() < 0.004)
            );
            assert!(
                matches!(parameter(17), Some(BassMessage::OverdriveDb(v)) if (v-drive).abs() < 0.15)
            );
        }
    }

    #[test]
    fn every_bank_key_has_a_distinct_clip_and_reassignment_recalls_the_slot() {
        let mut seq = Sequencer::INITIAL;
        assert_eq!(Slot::KEYS.map(|(key, _)| key), [0, 2, 4, 6, 7, 9, 11, 12]);
        assert_eq!(
            Slot::KEYS.map(|(_, slot)| slot.number()),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        for (part, channel) in Part::ALL
            .into_iter()
            .map(|part| (part, Channel::One))
            .chain([(Part::Bass, Channel::Two)])
        {
            let mut previous = Vec::new();
            for (_, slot) in Slot::KEYS {
                let clip = slot.clip(part, channel);
                assert!(!clip.pitches.is_empty());
                if part.is_kit() {
                    assert!(clip.pitches.iter().all(|&note| part.drum(note).is_some()));
                }
                assert!(!previous.contains(&clip.events));
                previous.push(clip.events);
            }
        }
        seq.choose(Part::Bass, Slot::KEYS[7].1);
        seq.choose(Part::Tr808, Slot::KEYS[1].1);
        assert_eq!(seq.slot(Channel::One, Part::Bass).number(), 8);
        assert_eq!(seq.slot(Channel::One, Part::Tr808).number(), 2);
        assert_eq!(seq.slot(Channel::Two, Part::Bass), Slot::FIRST);
    }

    #[test]
    fn queue_commits_at_the_same_audio_frame_across_tracks_and_other_phrases_continue() {
        let mut player = PatternPlayer::new((1000, 1));
        let tracks = Track::rack(Navigation::INITIAL.assignments);
        let play = Transport {
            bpm: 120,
            ..playing()
        };
        let tick = |p: &mut PatternPlayer, frames| {
            let mut wire = Vec::new();
            p.render(
                frames,
                play,
                &tracks,
                [true; 4],
                |_, _| Ok(()),
                |c, e| wire.push((c, e)),
            )
            .expect("host sink");
            wire
        };
        tick(&mut player, 1);
        player.requested[0] = Slot::KEYS[1].1;
        player.requested[2] = Slot::KEYS[2].1;
        tick(&mut player, 999);
        player.requested[0] = Slot::KEYS[6].1;
        player.requested[2] = Slot::FIRST; // cancel the pending drum switch
        tick(&mut player, 999);
        assert!(player.banks().iter().all(|b| b.slot == Slot::FIRST));
        let events = tick(&mut player, 2);
        assert!(events.iter().any(|(c, e)| *c == Channel::One
            && e.frame == 1
            && e.message == MidiMessage::Control(123, 0)));
        assert_eq!(player.banks().map(|b| b.slot.number()), [7, 1, 1, 1]);
        assert_eq!(player.banks().map(|b| b.bar), [0, 1, 1, 1]);
        player.requested[1] = Slot::KEYS[1].1;
        player.requested[2] = Slot::KEYS[7].1;
        tick(&mut player, 2000);
        assert_eq!(player.banks().map(|b| b.slot.number()), [7, 2, 8, 1]);
        assert_eq!(player.banks().map(|b| b.bar), [1, 0, 0, 2]);
        tick(&mut player, 2000);
        assert_eq!(player.banks().map(|b| b.bar), [0, 1, 0, 3]);
    }

    #[test]
    fn a_full_event_sink_is_an_explicit_failure() {
        let mut player = PatternPlayer::new((44_100, 1));
        assert_eq!(
            player.render(
                128,
                playing(),
                &Track::rack(Navigation::SYNTH_RACK.assignments),
                [true; 4],
                |_, _| Err(Error::Capacity),
                |_, _| ()
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
