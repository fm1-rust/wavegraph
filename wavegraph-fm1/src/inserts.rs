//! Two serial, independently assignable inserts per mixer strip.

use crate::{
    fx::{BassFx, INSERT_SLOTS, Insert, InsertKind, KeySource},
    instrument::{Message, Part},
    panel::{CHANNELS, Channel},
};
use heapless::Vec;
use wavegraph_dsp::{Event, Port, Process, ProcessContext};
use wavegraph_fx::{Message as FxMessage, Phaser, Saturator, SidechainDucker, ThreeBandEq};
use wavegraph_sample::I24;

#[derive(Clone, Copy)]
enum Processor {
    None,
    Compressor(SidechainDucker),
    Saturator(Saturator),
    Phaser(Phaser),
    Eq(ThreeBandEq),
}

impl Processor {
    fn kind(self) -> InsertKind {
        match self {
            Self::None => InsertKind::None,
            Self::Compressor(_) => InsertKind::Compressor,
            Self::Saturator(_) => InsertKind::Saturator,
            Self::Phaser(_) => InsertKind::Phaser,
            Self::Eq(_) => InsertKind::Eq,
        }
    }

    fn new(kind: InsertKind, rate_hz: u32, factory: SidechainDucker, eq: ThreeBandEq) -> Self {
        match kind {
            InsertKind::None => Self::None,
            InsertKind::Eq => Self::Eq(eq),
            InsertKind::Compressor => Self::Compressor(factory),
            InsertKind::Saturator => Self::Saturator(Saturator::new()),
            InsertKind::Phaser => Self::Phaser(Phaser::new(rate_hz).expect("audio sample rate")),
        }
    }
}

/// Each source supplies its full signal and isolated kick. The first four
/// outputs feed the strip faders. The last is scratch for the external key mix.
pub(super) struct ChannelInserts {
    parts: [Part; CHANNELS],
    controls: [[Insert; INSERT_SLOTS]; CHANNELS],
    processors: [[Processor; INSERT_SLOTS]; CHANNELS],
    rate_hz: u32,
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

fn compress(fx: &mut SidechainDucker, input: &[I24], key: &[I24], output: &mut [I24]) {
    fx.process(&mut ProcessContext {
        frames: input.len(),
        inputs: &[input, key],
        outputs: &mut [output],
        events: &[],
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

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "Scratch ranges partition a valid block"
    )]
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        // Routing and panel controls arrive together at the block boundary.
        let mut changed = [[false; INSERT_SLOTS]; CHANNELS];
        for event in cx.events {
            match event.message {
                Message::Insert(channel, part) => {
                    let i = channel.index();
                    self.parts[i] = part;
                    self.processors[i] = [Processor::None; INSERT_SLOTS];
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
        // Only the second compressor needs a copy of the preceding slot's
        // output. A small reusable scratch span avoids a new graph bus.
        let mut scratch = [I24::default(); 32];
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
                if processor.kind() != setting.kind() {
                    *processor =
                        Processor::new(setting.kind(), self.rate_hz, self.factory, self.flat_eq);
                    changed[i][slot] = true;
                }
                match (processor, setting) {
                    (Processor::Compressor(fx), Insert::Compressor(settings)) => {
                        if changed[i][slot] {
                            configure_compressor(fx, settings);
                        }
                        if slot == 0 {
                            compress(fx, source, key.unwrap_or(source), strip);
                        } else {
                            for (chunk, out) in strip.chunks_mut(scratch.len()).enumerate() {
                                let start = chunk * scratch.len();
                                let input = &mut scratch[..out.len()];
                                input.copy_from_slice(out);
                                let key = key.map_or(&*input, |key| &key[start..start + out.len()]);
                                compress(fx, input, key, out);
                            }
                        }
                    }
                    (Processor::Saturator(fx), Insert::Saturator(settings)) => {
                        if changed[i][slot] {
                            fx.set_controls(settings);
                        }
                        for sample in &mut *strip {
                            *sample = fx.tick(*sample);
                        }
                    }
                    (Processor::Phaser(fx), Insert::Phaser(settings)) => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_edits_and_bypass_only_affect_the_selected_strip() {
        let mut rack = ChannelInserts::new(44_118);
        let mut eq = InsertKind::Eq.initial();
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
            InsertKind::Saturator.initial(),
            InsertKind::Phaser.initial(),
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
