//! Route the 303 sidechain by instrument type as channel assignments change.

use crate::{
    fx::{BassFx, KeySource},
    instrument::{Message, Part},
    panel::{CHANNELS, Channel},
};
use heapless::Vec;
use wavegraph::{Event, Port, Process, ProcessContext};
use wavegraph_fx::{Message as FxMessage, SidechainDucker};
use wavegraph_sample::I24;

/// Each source supplies its full signal and isolated kick. The first four
/// outputs feed the strip faders. The last is scratch for the external key mix.
pub(super) struct BassInserts {
    parts: [Part; CHANNELS],
    controls: [BassFx; CHANNELS],
    duckers: [SidechainDucker; CHANNELS],
    factory: SidechainDucker,
}

impl BassInserts {
    pub(super) fn new(rate_hz: u32) -> Self {
        let factory = SidechainDucker::new(rate_hz, 32, 176).expect("valid 303 insert settings");
        Self {
            parts: [Part::Mono; CHANNELS],
            controls: [BassFx::INITIAL; CHANNELS],
            duckers: [factory; CHANNELS],
            factory,
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

impl Process<Message, I24> for BassInserts {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal; 2 * CHANNELS]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Signal; CHANNELS + 1]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        // Routing and panel controls arrive together at the block boundary.
        let mut changed = [false; CHANNELS];
        for event in cx.events {
            match event.message {
                Message::Insert(channel, part) => {
                    self.parts[channel.index()] = part;
                    self.duckers[channel.index()] = self.factory;
                    changed[channel.index()] = true;
                }
                Message::BassInsert(channel, settings) => {
                    self.controls[channel.index()] = settings;
                    changed[channel.index()] = true;
                }
                _ => (),
            }
        }
        let (strips, detector) = cx.outputs.split_at_mut(CHANNELS);
        let mut last_key = None;
        for channel in Channel::ALL {
            let i = channel.index();
            let source = cx.inputs[channel.source_bus()];
            if self.parts[i] == Part::Bass {
                let settings = self.controls[i];
                let key = if settings.key == KeySource::Bass {
                    source
                } else {
                    if last_key != Some(settings.key) {
                        self.detector(settings.key, cx.inputs, detector[0]);
                        last_key = Some(settings.key);
                    }
                    &*detector[0]
                };
                let controls = [
                    FxMessage::SidechainDepth(settings.depth),
                    FxMessage::SidechainHz(settings.split_hz),
                    FxMessage::Enabled(settings.enabled),
                ]
                .map(|message| Event {
                    frame: 0,
                    port: 0,
                    message,
                });
                self.duckers[i].process(&mut ProcessContext {
                    frames: cx.frames,
                    inputs: &[source, key],
                    outputs: &mut [strips[i]],
                    events: if changed[i] { &controls } else { &[] },
                    emit: &mut Err,
                });
            } else {
                strips[i].copy_from_slice(source);
            }
        }
    }
}
