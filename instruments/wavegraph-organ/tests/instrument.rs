//! Fixed-size polyphony, frame offsets, and PCM/f32 output.
#![allow(clippy::unwrap_used, reason = "Tests use known nonzero sample clocks")]
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_organ::{Adsr, Message, Organ, POLYPHONY, Params, Voicing};
use wavegraph_sample::I24;

fn event(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn render<const N: usize>(organ: &mut Organ, events: &[Event<Message>]) -> [i32; N] {
    let mut out = [I24::default(); N];
    organ.process(&mut ProcessContext {
        frames: N,
        inputs: &[],
        outputs: &mut [&mut out],
        events,
        emit: &mut Err,
    });
    out.map(I24::inner)
}

fn key(note: u8) -> Message {
    Message::NoteOn {
        note,
        velocity: 127,
    }
}

#[test]
fn voice_level_scales_the_polyphonic_sum_before_output_saturation() {
    let mut stock = Organ::new(44_118, 1).expect("sample clock");
    let mut loud = stock.clone().with_voice_level(255);
    let mut silent = stock.clone().with_voice_level(0);
    let chord = [60, 64, 67, 71].map(|note| event(0, key(note)));
    let dry = render::<2048>(&mut stock, &chord);
    let full = render::<2048>(&mut loud, &chord);
    assert_eq!(full, dry.map(|s| (s << 4).clamp(-(1 << 23), (1 << 23) - 1)));
    assert!(full.contains(&((1 << 23) - 1)));
    assert_eq!(render::<64>(&mut silent, &chord), [0; 64]);
}

#[test]
fn silent_until_note_and_event_offsets_are_exact() {
    assert!(Organ::new(0, 1).is_none());
    assert!(Organ::new(44_100, 0).is_none());
    let mut organ = Organ::new(24_000_000, 544).expect("nonzero sample clock");
    assert_eq!(render::<128>(&mut organ, &[]), [0; 128]);
    let block = render::<256>(
        &mut organ,
        &[event(64, key(60)), event(128, Message::NoteOff(60))],
    );
    assert!(block[..64].iter().all(|&s| s == 0));
    assert!(block[64..128].iter().any(|&s| s != 0));
    assert!(block[128..].iter().any(|&s| s != 0)); // smooth release
    assert_eq!(organ.held_notes(), [None; POLYPHONY]);
    for _ in 0..30 {
        render::<128>(&mut organ, &[]);
    }
    assert_eq!(organ.active_voices(), 0);
    assert_eq!(render::<128>(&mut organ, &[]), [0; 128]);
}

#[test]
fn four_independent_voices_and_deterministic_oldest_stealing() {
    let mut organ = Organ::new(48_000, 1).expect("nonzero sample clock");
    let chord = [60, 64, 67, 71].map(|note| event(0, key(note)));
    assert!(render::<128>(&mut organ, &chord).iter().any(|&s| s != 0));
    assert_eq!(organ.held_notes(), [Some(60), Some(64), Some(67), Some(71)]);
    assert_eq!(organ.active_voices(), POLYPHONY);
    render::<1>(&mut organ, &[event(0, key(72))]);
    assert_eq!(organ.held_notes(), [Some(72), Some(64), Some(67), Some(71)]);
    render::<1>(&mut organ, &[event(0, Message::NoteOff(60))]);
    assert_eq!(organ.held_notes()[0], Some(72)); // stolen note-off is harmless
    render::<1>(&mut organ, &[event(0, Message::NoteOff(67))]);
    assert_eq!(organ.held_notes()[2], None);
    render::<1>(&mut organ, &[event(0, key(75))]);
    assert_eq!(organ.held_notes(), [Some(72), Some(64), Some(75), Some(71)]);
    render::<1>(&mut organ, &[event(0, key(64))]);
    assert_eq!(
        organ
            .held_notes()
            .iter()
            .filter(|&&n| n == Some(64))
            .count(),
        1
    );
}

#[test]
fn a_split_block_is_bit_exact() {
    let mut whole = Organ::new(44_100, 1).expect("nonzero sample clock");
    let mut split = whole.clone();
    let events = [
        event(0, key(48)),
        event(31, key(55)),
        event(50, Message::Tone(125)),
        event(64, Message::NoteOff(48)),
        event(99, Message::DecayMs(20)),
    ];
    let full = render::<128>(&mut whole, &events);
    assert_eq!(full[..50], render::<50>(&mut split, &events[..2]));
    let middle = [
        event(0, Message::Tone(125)),
        event(14, Message::NoteOff(48)),
    ];
    assert_eq!(full[50..99], render::<49>(&mut split, &middle));
    assert_eq!(
        full[99..],
        render::<29>(&mut split, &[event(0, Message::DecayMs(20))])
    );
    assert_eq!(render::<64>(&mut whole, &[]), render::<64>(&mut split, &[]));
}

#[test]
fn transient_is_percussive_not_a_permanent_extra_oscillator() {
    let mut dry = Organ::new(44_100, 1).expect("nonzero sample clock");
    let mut loud = dry.clone();
    render::<0>(&mut dry, &[event(0, Message::Percussion(0))]);
    render::<0>(&mut loud, &[event(0, Message::Percussion(127))]);
    let a = render::<128>(&mut dry, &[event(0, key(60))]);
    let b = render::<128>(&mut loud, &[event(0, key(60))]);
    assert_ne!(a, b);
    for _ in 0..40 {
        render::<128>(&mut dry, &[]);
        render::<128>(&mut loud, &[]);
    }
    assert_eq!(render::<128>(&mut dry, &[]), render::<128>(&mut loud, &[]));
}

#[test]
fn parameter_clamps_reset_and_float_output() {
    let mut organ = Organ::new(48_000, 1).expect("nonzero sample clock");
    let messages = [
        event(0, Message::Tone(u8::MAX)),
        event(0, Message::Percussion(u8::MAX)),
        event(0, Message::DecayMs(0)),
        event(0, Message::ReleaseMs(u16::MAX)),
        event(0, key(56)),
    ];
    let pcm = render::<32>(&mut organ, &messages);
    assert!(pcm.iter().any(|&s| s != 0));
    assert_eq!(
        organ.params(),
        Params {
            voicing: Voicing::House,
            tone: 127,
            percussion: 127,
            decay_ms: 5,
            release_ms: 1000
        }
    );
    let mut out = [0.0_f32; 128];
    organ.process(&mut ProcessContext {
        frames: out.len(),
        inputs: &[],
        outputs: &mut [&mut out],
        events: &[],
        emit: &mut Err,
    });
    assert!(out.iter().any(|&x| x.abs() > 0.0));
    assert!(out.iter().all(|&x| x.abs() < 1.0));
    assert_eq!(organ.active_voices(), 1);
    render::<0>(&mut organ, &[event(0, Message::Reset)]);
    assert_eq!(organ.active_voices(), 0);
    assert_eq!(render::<128>(&mut organ, &[]), [0; 128]);
}

#[test]
fn all_notes_off_releases_and_immediate_zero_release_is_supported() {
    let mut organ = Organ::new(44_100, 1).expect("nonzero sample clock");
    render::<128>(&mut organ, &[event(0, key(60)), event(0, key(64))]);
    render::<0>(
        &mut organ,
        &[
            event(0, Message::ReleaseMs(0)),
            event(0, Message::AllNotesOff),
        ],
    );
    assert_eq!(organ.held_notes(), [None; POLYPHONY]);
    assert_eq!(render::<32>(&mut organ, &[]), [0; 32]);
}

#[test]
fn modeled_voicings_are_distinct_and_keep_note_state() {
    let mut house = Organ::new(44_100, 1).expect("audio clock");
    let mut analog = house.clone();
    render::<0>(&mut analog, &[event(0, Message::Voicing(Voicing::Analog))]);
    let h = render::<512>(&mut house, &[event(0, key(48))]);
    let a = render::<512>(&mut analog, &[event(0, key(48))]);
    assert_ne!(h, a);
    assert_eq!(house.held_notes(), analog.held_notes());
}

#[test]
fn vca_shapes_the_body_and_click_at_the_event_frame() {
    let mut organ = Organ::new(1_000, 1).expect("one frame per millisecond");
    let mut reference = organ.clone();
    let adsr = Adsr {
        attack_us: 4_000,
        decay_us: 4_000,
        sustain_percent: 50,
        release_us: 4_000,
    };
    let samples = render::<16>(
        &mut organ,
        &[
            event(2, Message::VcaEnvelope(adsr)),
            event(2, key(24)),
            event(10, Message::NoteOff(24)),
        ],
    );
    let full = render::<16>(
        &mut reference,
        &[
            event(2, Message::VcaEnvelope(Adsr::GATE)),
            event(2, key(24)),
        ],
    );
    // Expected gain in 32nds, including quadratic decay and release.
    let gains = [0, 0, 8, 16, 24, 32, 25, 20, 17, 16, 9, 4, 1, 0, 0, 0];
    for ((sample, dry), gain) in samples.into_iter().zip(full).zip(gains) {
        // One Q15 unit of gain rounding plus one integer mixer LSB.
        assert!((32 * sample - gain * dry).abs() <= 32 * 32);
    }
    assert_eq!(&samples[13..], &[0; 3]);
    assert_eq!(organ.active_voices(), 0);
}

#[test]
fn a_held_silent_voice_can_swell_again_and_edits_retime_its_release() {
    let mut organ = Organ::new(1_000, 1).expect("audio clock");
    let mut adsr = Adsr {
        sustain_percent: 0,
        release_us: 8_000,
        ..Adsr::GATE
    };
    assert_eq!(
        render::<8>(
            &mut organ,
            &[event(0, Message::VcaEnvelope(adsr)), event(0, key(24))]
        ),
        [0; 8],
    );
    assert_eq!(organ.held_notes()[0], Some(24));
    assert_eq!(organ.active_voices(), 1);
    adsr.sustain_percent = 100;
    assert!(
        render::<8>(&mut organ, &[event(0, Message::VcaEnvelope(adsr))])
            .iter()
            .any(|&s| s != 0)
    );
    render::<2>(&mut organ, &[event(0, Message::AllNotesOff)]);
    // Six of eight release frames remain. Halving the release leaves three.
    let tail = render::<5>(&mut organ, &[event(0, Message::ReleaseMs(4))]);
    assert_ne!(tail[1], 0);
    assert_eq!(tail[2..], [0; 3]);
    assert_eq!(organ.active_voices(), 0);
    assert_eq!(
        organ.vca_envelope(),
        Adsr {
            release_us: 4_000,
            ..adsr
        }
    );
    assert_eq!(organ.params().release_ms, 4);
}

#[test]
fn overlapping_keys_run_independent_envelopes_and_restrikes_start_fresh() {
    let mut organ = Organ::new(8_000, 1).expect("audio clock");
    let adsr = Adsr {
        attack_us: 2_000,
        decay_us: 3_000,
        sustain_percent: 35,
        release_us: 4_000,
    };
    render::<0>(&mut organ, &[event(0, Message::VcaEnvelope(adsr))]);
    let mut first = organ.clone();
    let mut second = organ.clone();
    let a = [event(2, key(36)), event(14, Message::NoteOff(36))];
    let b = [event(8, key(48))];
    let chord = render::<64>(&mut organ, &[a[0], b[0], a[1]]);
    let left = render::<64>(&mut first, &a);
    let right = render::<64>(&mut second, &b);
    assert_eq!(chord, core::array::from_fn(|i| left[i] + right[i]));
    assert_eq!(organ.active_voices(), 1);
    let strike = [event(0, key(48))];
    // Compare a re-strike on a held note to a first strike after Reset.
    render::<0>(&mut first, &[event(0, Message::Reset)]);
    assert_eq!(
        render::<32>(&mut organ, &strike),
        render::<32>(&mut first, &strike)
    );
}
