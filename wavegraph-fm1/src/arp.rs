//! Channel-local panel settings and routing for the live MIDI arpeggiator.

use crate::{
    instrument::{Message, RoutedEvents, Track},
    sequencer::DrumMessage,
};
use wavegraph_arp::{Chord, Clock, Config, Division, Mode};
use wavegraph_dsp::{Event, bytecode::Error, midi::MidiMessage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Controls {
    pub(super) enabled: bool,
    pub(super) config: Config,
}

impl Controls {
    pub(super) const INITIAL: Self = Self {
        enabled: false,
        config: Config::INITIAL,
    };
    const MODES: [Mode; 5] = [
        Mode::Up,
        Mode::Down,
        Mode::UpDown,
        Mode::AsPlayed,
        Mode::Random,
    ];
    const RATES: [Division; 6] = [
        Division::Quarter,
        Division::Eighth,
        Division::EighthTriplet,
        Division::Sixteenth,
        Division::SixteenthTriplet,
        Division::ThirtySecond,
    ];

    pub(super) fn edit(&mut self, select: i8, knobs: [i8; 4], sel: bool) {
        let [mode, rate, octaves, gate] = knobs;
        if sel {
            self.enabled = !self.enabled;
        }
        if select != 0 {
            self.config.latch = select > 0;
        }
        self.config.mode = turn(&Self::MODES, self.config.mode, mode);
        self.config.division = turn(&Self::RATES, self.config.division, rate);
        self.config.octaves = self
            .config
            .octaves
            .saturating_add_signed(octaves)
            .clamp(1, 4);
        self.config.gate = self.config.gate.saturating_add_signed(gate).clamp(1, 100);
    }
}

fn turn<T: Copy + PartialEq>(values: &[T], current: T, detents: i8) -> T {
    let i = values
        .iter()
        .position(|v| *v == current)
        .expect("complete menu");
    values[i
        .saturating_add_signed(isize::from(detents))
        .min(values.len().saturating_sub(1))]
}

pub(super) struct ChannelArp {
    pub(super) chord: Chord,
    pub(super) clock: Clock,
}

impl ChannelArp {
    pub(super) fn new(rate: (u32, u32)) -> Self {
        Self {
            chord: Chord::new(),
            clock: Clock::new(rate.0, rate.1).expect("FM-1 audio clock"),
        }
    }
}

/// Translate the generated event once for both the graph inlet and the USB outlet.
pub(super) fn route(
    track: &Track,
    event: Event<MidiMessage>,
    events: &mut RoutedEvents,
    mut wire: impl FnMut(Event<MidiMessage>),
) -> Result<(), Error> {
    let part = track.assignment.part;
    let transpose = |key: u8| key.saturating_add_signed(track.key_transpose()).min(127);
    let (midi, graph) = match event.message {
        MidiMessage::On(mut n) => {
            n.number = transpose(n.number);
            let graph = if part.is_kit() {
                part.drum(n.number)
                    .map(|v| Message::Drum(DrumMessage::Trigger(v, n.velocity)))
            } else {
                Some(part.note_on(n, false))
            };
            (MidiMessage::On(n), graph)
        }
        MidiMessage::Off(n) => {
            let n = transpose(n);
            (
                MidiMessage::Off(n),
                (!part.is_kit()).then(|| part.note_off(n)),
            )
        }
        MidiMessage::Control(..) => (event.message, None),
    };
    if let Some(message) = graph {
        events
            .push(Event {
                frame: event.frame,
                port: 0,
                message,
            })
            .map_err(|_| Error::Capacity)?;
    }
    wire(Event {
        message: midi,
        ..event
    });
    Ok(())
}
