//! Exercise the public patch object at event, gate and DMA boundaries.
use wavegraph_arp::{Arpeggiator, Chord, Clock, Config, Division, Mode};
use wavegraph_core::{
    Event, Port, Process, ProcessContext,
    midi::{MidiMessage, Note},
};

fn on(key: u8, velocity: u8) -> MidiMessage {
    MidiMessage::On(Note {
        number: key,
        velocity,
    })
}
fn at(frame: usize, message: MidiMessage) -> Event<MidiMessage> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn block(
    arp: &mut Arpeggiator,
    frames: usize,
    input: &[Event<MidiMessage>],
) -> Vec<Event<MidiMessage>> {
    let mut out = Vec::new();
    arp.process(&mut ProcessContext::<_, f32> {
        frames,
        inputs: &[],
        outputs: &mut [],
        events: input,
        emit: &mut |event| {
            out.push(event);
            Ok(())
        },
    });
    out
}

#[test]
fn modes_velocity_and_full_gate_at_a_frame_offset() {
    let chord = [at(7, on(64, 92)), at(7, on(60, 80)), at(7, on(67, 111))];
    for (mode, keys) in [
        (Mode::Up, [60, 64, 67, 72, 76, 79, 60]),
        (Mode::Down, [79, 76, 72, 67, 64, 60, 79]),
        (Mode::UpDown, [60, 64, 67, 72, 76, 79, 76]),
        (Mode::AsPlayed, [64, 60, 67, 76, 72, 79, 64]),
    ] {
        let mut arp = Arpeggiator::new(1000, 1).expect("test clock");
        assert_eq!(
            <Arpeggiator as Process<MidiMessage>>::inlets(&arp),
            [Port::Message]
        );
        assert_eq!(
            <Arpeggiator as Process<MidiMessage>>::outlets(&arp),
            [Port::Message]
        );
        arp.configure(
            Config {
                mode,
                octaves: 2,
                gate: 100,
                ..Config::INITIAL
            },
            120,
        );
        let result = block(&mut arp, 758, &chord);
        let mut expected = Vec::new();
        for (i, key) in keys.into_iter().enumerate() {
            let frame = 7 + i * 125;
            if i != 0 {
                expected.push(at(frame, MidiMessage::Off(keys[i - 1])));
            }
            let velocity = match key % 12 {
                0 => 80,
                4 => 92,
                _ => 111,
            };
            expected.push(at(frame, on(key, velocity)));
        }
        assert_eq!(result, expected);
    }
}

#[test]
fn latch_replaces_a_released_chord_and_panic_forwards_after_note_off() {
    let mut arp = Arpeggiator::new(1000, 1).expect("test clock");
    let config = Config {
        latch: true,
        gate: 50,
        ..Config::INITIAL
    };
    arp.configure(config, 120);
    assert_eq!(
        block(
            &mut arp,
            126,
            &[
                at(0, on(60, 81)),
                at(0, on(67, 100)),
                at(21, MidiMessage::Off(60)),
                at(21, on(67, 0)),
                at(75, MidiMessage::Control(74, 99))
            ]
        ),
        [
            at(0, on(60, 81)),
            at(62, MidiMessage::Off(60)),
            at(75, MidiMessage::Control(74, 99)),
            at(125, on(67, 100))
        ]
    );
    assert_eq!(
        block(
            &mut arp,
            8,
            &[at(3, on(70, 40)), at(5, MidiMessage::Off(70))]
        ),
        [at(3, MidiMessage::Off(67)), at(3, on(70, 40))]
    );
    arp.configure(
        Config {
            latch: false,
            ..config
        },
        120,
    );
    assert_eq!(block(&mut arp, 0, &[]), [at(0, MidiMessage::Off(70))]);
    for cc in [120, 123] {
        arp.configure(config, 120);
        assert_eq!(
            block(
                &mut arp,
                200,
                &[
                    at(2, on(50, 64)),
                    at(3, MidiMessage::Off(50)),
                    at(30, MidiMessage::Control(cc, 0))
                ]
            ),
            [
                at(2, on(50, 64)),
                at(30, MidiMessage::Off(50)),
                at(30, MidiMessage::Control(cc, 0))
            ]
        );
    }
}

#[test]
fn rational_timing_survives_irregular_blocks_and_skips_unplayable_octaves() {
    let make = || {
        let mut arp = Arpeggiator::new(24_000_000, 544).expect("FM-1 clock");
        arp.configure(
            Config {
                mode: Mode::Down,
                division: Division::SixteenthTriplet,
                octaves: 4,
                gate: 72,
                latch: false,
            },
            137,
        );
        arp
    };
    let input = [at(0, on(120, 74)), at(0, on(127, 35))];
    let mut arp = make();
    let expected = block(&mut arp, 120_000, &input);
    let mut chunks = make();
    let mut actual = Vec::new();
    let mut base = 0;
    for len in [128, 1, 93, 256, 17].into_iter().cycle() {
        if base == 120_000 {
            break;
        }
        let frames = len.min(120_000 - base);
        let events = block(&mut chunks, frames, if base == 0 { &input } else { &[] });
        actual.extend(events.into_iter().map(|e| at(base + e.frame, e.message)));
        base += frames;
    }
    assert_eq!(actual, expected);
    for (i, e) in expected
        .iter()
        .filter(|e| matches!(e.message, MidiMessage::On(_)))
        .enumerate()
    {
        let expected_frame = i as u64 * 24_000_000 * 60 / (544 * 137 * 6);
        assert_eq!(e.frame as u64, expected_frame);
        assert_eq!(
            e.message,
            if i % 2 == 0 { on(127, 35) } else { on(120, 74) }
        );
    }
}

#[test]
fn random_picks_only_valid_notes_and_rejected_releases_are_retried() {
    let mut arp = Arpeggiator::new(1000, 1).expect("test clock");
    arp.configure(
        Config {
            mode: Mode::Random,
            gate: 100,
            octaves: 4,
            ..Config::INITIAL
        },
        120,
    );
    let result = block(&mut arp, 3000, &[at(0, on(116, 100)), at(0, on(119, 90))]);
    assert!(result.iter().any(|e| e.message == on(116, 100)));
    assert!(result.iter().any(|e| e.message == on(119, 90)));
    assert!(
        result
            .iter()
            .all(|e| matches!(e.message, MidiMessage::Off(116 | 119))
                || e.message == on(116, 100)
                || e.message == on(119, 90))
    );
    let mut rejected = None;
    arp.process(&mut ProcessContext::<_, f32> {
        frames: 1,
        inputs: &[],
        outputs: &mut [],
        events: &[],
        emit: &mut |e| {
            rejected = Some(e);
            Err(e)
        },
    });
    assert!(matches!(
        rejected,
        Some(Event {
            message: MidiMessage::Off(_),
            ..
        })
    ));
    assert_eq!(block(&mut arp, 0, &[]).first(), rejected.as_ref());
}

#[test]
fn all_midi_pitches_fit_and_each_octave_skips_only_out_of_range_notes() {
    let mut arp = Arpeggiator::new(1000, 1).expect("test clock");
    arp.configure(
        Config {
            mode: Mode::AsPlayed,
            octaves: 4,
            ..Config::INITIAL
        },
        120,
    );
    let input: Vec<_> = (0u8..128)
        .rev()
        .map(|key| at(0, on(key, key.max(1))))
        .collect();
    let expected: Vec<_> = (0u8..4)
        .flat_map(|octave| {
            let shift = octave * 12;
            (0..128 - shift)
                .rev()
                .map(move |key| on(key + shift, key.max(1)))
        })
        .collect();
    let output = block(&mut arp, expected.len() * 125, &input);
    let strikes: Vec<_> = output
        .iter()
        .filter(|e| matches!(e.message, MidiMessage::On(_)))
        .map(|e| e.message)
        .collect();
    assert_eq!(strikes, expected);
}

#[test]
fn crossing_a_tempo_edit_finishes_the_current_step_and_a_release_wins_at_the_next_boundary() {
    let mut arp = Arpeggiator::new(1000, 1).expect("test clock");
    arp.configure(
        Config {
            division: Division::Quarter,
            gate: 100,
            ..Config::INITIAL
        },
        60,
    );
    assert_eq!(
        block(&mut arp, 100, &[at(0, on(57, 90))]),
        [at(0, on(57, 90))]
    );
    arp.configure(
        Config {
            division: Division::Eighth,
            gate: 50,
            ..Config::INITIAL
        },
        120,
    );
    assert_eq!(
        block(&mut arp, 1000, &[]),
        [at(900, MidiMessage::Off(57)), at(900, on(57, 90))]
    );
    assert_eq!(
        block(&mut arp, 500, &[at(150, MidiMessage::Off(57))]),
        [at(25, MidiMessage::Off(57))]
    );
}

#[test]
fn skipped_steps_release_the_old_gate_and_preserve_the_next_attack() {
    let mut chord = Chord::new();
    for n in [60, 64, 67] {
        chord.input(on(n, 88), false);
    }
    let mut clock = Clock::new(1000, 1).expect("test sample clock");
    let config = Config::INITIAL;
    clock
        .render(&chord, config, 120, 10, |_| Ok::<_, ()>(()))
        .expect("first attack");
    assert_eq!(clock.skip(&chord, config, 120, 0), None);
    assert_eq!(clock.sounding(), Some(60));
    assert_eq!(clock.skip(&chord, config, 120, 260), Some(60));
    assert_eq!(clock.sounding(), None);
    let mut events = Vec::new();
    clock
        .render(&chord, config, 120, 108, |e| {
            events.push(e);
            Ok::<_, ()>(())
        })
        .expect("next step");
    assert_eq!(events, [at(105, on(60, 88))]);
}
