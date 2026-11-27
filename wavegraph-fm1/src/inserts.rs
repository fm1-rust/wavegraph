//! Two serial, independently assignable inserts per mixer strip.

use crate::{
    fx::{BassFx, INSERT_SLOTS, Insert, InsertKind, KeySource},
    instrument::{DRY_HEADROOM, Message, Part},
    panel::{CHANNELS, Channel},
};
use heapless::Vec;
use wavegraph_dsp::{Event, Port, Process, ProcessContext};
use wavegraph_fx::{
    Chorus, Flanger, Message as FxMessage, Phaser, Saturator, SidechainDucker, Slicer, ThreeBandEq,
};
use wavegraph_sample::I24;

mod filter;
use filter::ClockedFilter;

#[allow(
    clippy::large_enum_variant,
    reason = "Each assignable slot owns its delay storage without a heap."
)]
#[derive(Clone, Copy)]
enum Processor {
    None,
    Compressor(SidechainDucker),
    Saturator(Saturator),
    Phaser(Phaser),
    Chorus(Chorus),
    Flanger(Flanger),
    Eq(ThreeBandEq),
}

impl Processor {
    fn kind(&self) -> InsertKind {
        match self {
            Self::None => InsertKind::None,
            Self::Compressor(_) => InsertKind::Compressor,
            Self::Saturator(_) => InsertKind::Saturator,
            Self::Phaser(_) => InsertKind::Phaser,
            Self::Flanger(_) => InsertKind::Flanger,
            Self::Chorus(_) => InsertKind::Chorus,
            Self::Eq(_) => InsertKind::Eq,
        }
    }

    // Construct directly in the slot. A chorus-sized temporary adds work to
    // small Saturator and Phaser recalls on the audio thread.
    fn replace(&mut self, kind: InsertKind, rate: u32, factory: SidechainDucker, eq: ThreeBandEq) {
        match kind {
            InsertKind::None => *self = Self::None,
            #[allow(clippy::cast_precision_loss)]
            InsertKind::Chorus => *self = Self::Chorus(Chorus::new(1.0 / rate as f32)),
            InsertKind::Eq => *self = Self::Eq(eq),
            InsertKind::Compressor => *self = Self::Compressor(factory),
            InsertKind::Saturator => *self = Self::Saturator(Saturator::new()),
            InsertKind::Phaser => {
                *self = Self::Phaser(Phaser::new(rate).expect("audio sample rate"))
            }
            InsertKind::Flanger => {
                *self = Self::Flanger(Flanger::new(rate).expect("audio sample rate"))
            }
        }
    }
}

/// Each source supplies its full signal and isolated kick. The first four
/// outputs feed the strip faders. After the external key mix, the last outlet
/// carries post-fader Hoover stereo width when a clean Hoover is routed.
pub(super) struct ChannelInserts {
    parts: [Part; CHANNELS],
    controls: [[Insert; INSERT_SLOTS]; CHANNELS],
    processors: [[Processor; INSERT_SLOTS]; CHANNELS],
    rate_hz: u32,
    beat_flanger: Flanger,
    slicers: [Slicer; CHANNELS],
    filters: [ClockedFilter; CHANNELS],
    beat_target: Option<Channel>,
    hoover_levels: [u8; CHANNELS],
    check_dry: u8,
    factory: SidechainDucker,
    flat_eq: ThreeBandEq,
}

impl ChannelInserts {
    pub(super) fn new(rate_hz: u32) -> Self {
        let factory = SidechainDucker::new(rate_hz, 32, 176).expect("valid insert settings");
        Self {
            parts: [Part::Mono; CHANNELS],
            controls: [[Insert::None; INSERT_SLOTS]; CHANNELS],
            processors: [[Processor::None; INSERT_SLOTS]; CHANNELS],
            rate_hz,
            beat_flanger: Flanger::new(rate_hz).expect("audio sample rate"),
            beat_target: None,
            hoover_levels: [0; CHANNELS],
            check_dry: 0,
            slicers: [Slicer::new(rate_hz).expect("audio sample rate"); CHANNELS],
            filters: [ClockedFilter::new(rate_hz); CHANNELS],
            factory,
            flat_eq: ThreeBandEq::new(rate_hz).expect("audio sample rate"),
        }
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The sum of up to four Q23 signals fits i32."
    )]
    fn detector(&self, key: KeySource, inputs: &[&[I24]], output: &mut [I24]) {
        let mut sources = Vec::<_, CHANNELS>::new();
        for (part, pair) in self.parts.iter().zip(inputs.as_chunks::<2>().0) {
            let source = match (key, part) {
                (KeySource::Kick, Part::Tr909) | (KeySource::Kick808, Part::Tr808) => pair[1],
                (KeySource::Kit, Part::Tr909)
                | (KeySource::Kit808, Part::Tr808)
                | (KeySource::Organ, Part::Organ)
                | (KeySource::Mono, Part::Mono) => pair[0],
                _ => continue,
            };
            sources.push(source).expect("one key per strip");
        }
        match sources.as_slice() {
            [] => output.fill(I24::default()),
            [one] => output.copy_from_slice(one),
            sources => {
                for (i, sample) in output.iter_mut().enumerate() {
                    let sum: i32 = sources.iter().map(|source| source[i].inner()).sum();
                    *sample = I24::new_unchecked(sum.clamp(-(1 << 23), (1 << 23) - 1));
                }
            }
        }
    }
    // The detector scratch is free once the inserts finish. Its new contents
    // are for the stereo master alone, leaving sends and tape capture mono.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Four Q23 samples scaled by byte faders fit i32 after the Q7 shift"
    )]
    fn mix_hoover_side(&mut self, inputs: &[&[I24]], strips: &[&mut [I24]], out: &mut [I24]) {
        let mut taps = Vec::<_, CHANNELS>::new();
        for channel in Channel::ALL {
            let i = channel.index();
            let gain = self.hoover_levels[i];
            let src = channel.source_bus();
            // Bypassed filters, flangers and gates can still be fading to dry.
            if gain == 0 {
                continue;
            }
            let bit = 1 << i;
            if self.check_dry & bit != 0 {
                if *strips[i] != *inputs[src] {
                    continue;
                }
                self.check_dry &= !bit;
            }
            taps.push((i32::from(gain), inputs[src + 1]))
                .expect("one side per strip");
        }
        for (frame, out) in out.iter_mut().enumerate() {
            let sum: i32 = taps
                .iter()
                .map(|(gain, tap)| (tap[frame].inner() * gain) >> 7)
                .sum();
            *out = I24::new_unchecked(sum >> DRY_HEADROOM);
        }
    }
}

fn configure_compressor(fx: &mut SidechainDucker, settings: BassFx) {
    let events = [
        FxMessage::SidechainDepth(settings.depth),
        FxMessage::SidechainHz(settings.split_hz),
        FxMessage::Enabled(settings.enabled),
    ]
    .map(|message| Event {
        frame: 0,
        port: 0,
        message,
    });
    fx.process(&mut ProcessContext {
        frames: 0,
        inputs: &[&[], &[]],
        outputs: &mut [&mut []],
        events: &events,
        emit: &mut Err,
    });
}

impl Process<Message, I24> for ChannelInserts {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal; 2 * CHANNELS]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal; CHANNELS + 1]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        // Routing and panel controls arrive together at the block boundary.
        let mut beat = None;
        let mut filter_clock = wavegraph_lfo::Phase::default();
        let mut restart_filters = false;
        let mut changed = [[false; INSERT_SLOTS]; CHANNELS];
        for event in cx.events {
            match event.message {
                Message::HooverLevels(levels) => {
                    for (i, (&old, &new)) in self.hoover_levels.iter().zip(&levels).enumerate() {
                        if old == 0 && new > 0 {
                            self.check_dry |= 1 << i;
                        }
                    }
                    self.hoover_levels = levels;
                }
                Message::FilterClock(timing, restart) => {
                    filter_clock.control(timing);
                    restart_filters = restart;
                }
                Message::BeatFilter(channel, settings) => {
                    self.filters[channel.index()].configure(settings)
                }
                Message::BeatSlicer(channel, controls, pattern, timing) => {
                    let slicer = &mut self.slicers[channel.index()];
                    slicer.set_controls(controls);
                    slicer.set_pattern(pattern);
                    slicer.set_phase(timing);
                }
                Message::BeatFlanger(channel, controls, timing) => {
                    beat = Some((channel, controls, timing))
                }
                Message::Insert(channel, part) => {
                    let i = channel.index();
                    self.parts[i] = part;
                    self.processors[i].fill(Processor::None);
                    changed[i] = [true; INSERT_SLOTS];
                }
                Message::ChannelInserts(channel, settings) => {
                    let i = channel.index();
                    for (slot, setting) in settings.into_iter().enumerate() {
                        changed[i][slot] |= self.controls[i][slot] != setting;
                        self.controls[i][slot] = setting;
                    }
                }
                _ => (),
            }
        }
        let (strips, detector) = cx.outputs.split_at_mut(CHANNELS);
        let mut last_key = None;
        for channel in Channel::ALL {
            let i = channel.index();
            let source = cx.inputs[channel.source_bus()];
            let strip = &mut *strips[i];
            strip.copy_from_slice(source);
            for slot in 0..INSERT_SLOTS {
                let setting = self.controls[i][slot];
                let key = if let Insert::Compressor(settings) = setting {
                    if settings.key == KeySource::Bass {
                        None // Self-key from this slot's input, not the raw source.
                    } else {
                        if last_key != Some(settings.key) {
                            self.detector(settings.key, cx.inputs, detector[0]);
                            last_key = Some(settings.key);
                        }
                        Some(&*detector[0])
                    }
                } else {
                    None
                };
                let processor = &mut self.processors[i][slot];
                if changed[i][slot] && processor.kind() != setting.kind() {
                    processor.replace(setting.kind(), self.rate_hz, self.factory, self.flat_eq);
                }
                match (processor, setting) {
                    (Processor::Compressor(fx), Insert::Compressor(settings)) => {
                        if changed[i][slot] {
                            configure_compressor(fx, settings);
                        }
                        fx.process_in_place(strip, key);
                    }
                    (Processor::Saturator(fx), Insert::Saturator(settings)) => {
                        if changed[i][slot] {
                            fx.set_controls(settings);
                        }
                        fx.process_in_place(strip);
                    }
                    (Processor::Phaser(fx), Insert::Phaser(settings)) => {
                        if changed[i][slot] {
                            fx.set_controls(settings);
                        }
                        fx.process_in_place(strip);
                    }
                    (Processor::Flanger(fx), Insert::Flanger(settings)) => {
                        if changed[i][slot] {
                            fx.set_controls(settings);
                        }
                        for sample in &mut *strip {
                            *sample = fx.tick(*sample);
                        }
                    }
                    (Processor::Chorus(fx), Insert::Chorus(settings)) => {
                        if changed[i][slot] {
                            fx.set_controls(settings);
                        }
                        for sample in &mut *strip {
                            *sample = fx.tick(*sample);
                        }
                    }
                    (Processor::Eq(fx), Insert::Eq(settings)) => {
                        if changed[i][slot] {
                            fx.set_controls(settings);
                        }
                        for sample in &mut *strip {
                            *sample = fx.tick(*sample);
                        }
                    }
                    _ => (),
                }
            }
        }
        if let Some(old) = self.beat_target
            && beat.is_none_or(|b| b.0 != old)
        {
            self.beat_flanger.bypass();
            for sample in &mut *strips[old.index()] {
                *sample = self.beat_flanger.tick(*sample);
            }
            if self.beat_flanger.is_dry() || beat.is_some() {
                self.beat_target = None;
            }
        }
        if let Some((channel, controls, timing)) = beat {
            if self.beat_target != Some(channel) {
                self.beat_flanger = Flanger::new(self.rate_hz).expect("audio sample rate");
                self.beat_target = Some(channel);
            }
            self.beat_flanger.set_controls(controls);
            self.beat_flanger.set_phase(timing);
            for sample in &mut *strips[channel.index()] {
                *sample = self.beat_flanger.tick(*sample);
            }
        }
        for ((filter, slicer), strip) in self
            .filters
            .iter_mut()
            .zip(&mut self.slicers)
            .zip(&mut *strips)
        {
            filter.process(strip, filter_clock, restart_filters);
            slicer.process_in_place(strip);
        }
        if self.hoover_levels != [0; CHANNELS] {
            self.mix_hoover_side(cx.inputs, strips, detector[0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wavegraph_fx::FlangerControls;

    #[test]
    fn flangers_in_both_slots_and_on_other_strips_have_independent_delay_lines() {
        let mut rack = ChannelInserts::new(44_118);
        // A fully wet, stationary tap delays by 32 frames at the board rate.
        let flange = Insert::Flanger(FlangerControls {
            depth: 0,
            feedback: 0,
            mix: 128,
            ..FlangerControls::INITIAL
        });
        let settings = [
            (Channel::One, [flange, flange]),
            (Channel::Two, [Insert::None, flange]),
            (Channel::Three, [flange, Insert::None]),
        ]
        .map(|(channel, slots)| Event {
            frame: 0,
            port: 0,
            message: Message::ChannelInserts(channel, slots),
        });
        let mut input = [I24::default(); 128];
        let silence = input;
        let mut outputs = [[I24::default(); 128]; CHANNELS + 1];
        for warmup in [true, false] {
            if !warmup {
                input[3] = I24::new_unchecked(1 << 20);
            }
            rack.process(&mut ProcessContext {
                frames: input.len(),
                inputs: &[
                    &input, &silence, &silence, &silence, &input, &silence, &input, &silence,
                ],
                outputs: &mut outputs.each_mut().map(|block| &mut block[..]),
                events: if warmup { &settings } else { &[] },
                emit: &mut Err,
            });
        }
        assert_eq!(outputs[1], silence);
        assert_eq!(outputs[3], input);
        input.rotate_right(32);
        assert_eq!(outputs[2], input);
        input.rotate_right(32);
        assert_eq!(outputs[0], input);
    }

    #[test]
    fn eq_edits_and_bypass_only_affect_the_selected_strip() {
        let mut rack = ChannelInserts::new(44_118);
        let mut eq = InsertKind::Eq.initial(Part::Mono);
        let input = [I24::new_unchecked(100_000); 128];
        let silence = [I24::default(); 128];
        let mut outputs = [[I24::default(); 128]; CHANNELS + 1];

        // Put the EQ in channel 2's second slot. Equal gain in every band
        // has a known transfer even before the crossover filters settle.
        for (knobs, toggle, expected) in [
            ([0; 4], false, 100_000),
            ([8; 4], false, 251_188),
            ([0; 4], true, 100_000),
            ([0; 4], true, 251_188),
            ([-127; 4], false, 25_118),
        ] {
            eq.edit(knobs, toggle);
            rack.process(&mut ProcessContext {
                frames: input.len(),
                inputs: &[
                    &input, &silence, &input, &silence, &input, &silence, &input, &silence,
                ],
                outputs: &mut outputs.each_mut().map(|block| &mut block[..]),
                events: &[Event {
                    frame: 0,
                    port: 0,
                    message: Message::ChannelInserts(Channel::Two, [Insert::None, eq]),
                }],
                emit: &mut Err,
            });
            for sample in &outputs[1][63..] {
                assert_eq!(sample.inner(), expected);
            }
            if expected != input[0].inner() {
                assert_ne!(outputs[1][0].inner(), expected, "gain changes ramp");
            }
            for dry in [0, 2, 3] {
                assert_eq!(outputs[dry], input);
            }
        }
    }

    #[test]
    fn slots_are_serial_and_other_channels_are_dry() {
        let mut rack = ChannelInserts::new(44_118);
        let settings = [
            InsertKind::Saturator.initial(Part::Mono),
            InsertKind::Phaser.initial(Part::Mono),
        ];
        let event = [Event {
            frame: 0,
            port: 0,
            message: Message::ChannelInserts(Channel::One, settings),
        }];
        let input = core::array::from_fn::<_, 128, _>(|i| {
            I24::new_unchecked(if i % 32 < 16 { 1 << 20 } else { -(1 << 20) })
        });
        let silent = [I24::default(); 128];
        let mut outputs = [[I24::default(); 128]; CHANNELS + 1];
        let mut saturator = Saturator::new();
        let mut phaser = Phaser::new(44_118).expect("sample rate");
        for block in 0..4 {
            let mut out = outputs.each_mut().map(|slice| &mut slice[..]);
            rack.process(&mut ProcessContext {
                frames: 128,
                inputs: &[
                    &input, &silent, &input, &silent, &input, &silent, &input, &silent,
                ],
                outputs: &mut out,
                events: if block == 0 { &event } else { &[] },
                emit: &mut Err,
            });
            let expected = input.map(|sample| phaser.tick(saturator.tick(sample)));
            assert_eq!(outputs[0], expected);
            assert_eq!(outputs[1], input);
            assert_eq!(outputs[2], input);
            assert_eq!(outputs[3], input);
        }
    }
}
