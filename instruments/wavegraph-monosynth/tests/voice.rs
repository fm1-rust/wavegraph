//! Check pitch, waveform levels and note handling at exact frame offsets.
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_monosynth::{Adsr, Message, Monosynth, Tuning, Waveform};
use wavegraph_sample::I24;

fn render<const FRAMES: usize>(
    voice: &mut Monosynth<'_>,
    events: &[Event<Message>],
) -> [i32; FRAMES] {
    let mut samples = [I24::default(); FRAMES];
    voice.process(&mut ProcessContext {
        frames: FRAMES,
        inputs: &[],
        outputs: &mut [&mut samples],
        events,
        emit: &mut Err,
    });
    samples.map(I24::inner)
}

fn at(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

#[test]
fn cutoff_tracks_the_half_power_frequency_at_each_sample_rate() {
    fn gain(tuning: &Tuning, note: u8, cutoff: u16) -> f64 {
        let mut dry = Monosynth::new(tuning);
        let mut filtered = dry.clone();
        let note_on = at(
            0,
            Message::NoteOn {
                note,
                velocity: 127,
            },
        );
        render::<2048>(&mut dry, &[note_on]);
        render::<2048>(&mut filtered, &[note_on, at(0, Message::CutoffHz(cutoff))]);
        let energy = |samples: [i32; 4096]| {
            samples
                .into_iter()
                .map(|s| f64::from(s).powi(2))
                .sum::<f64>()
        };
        (energy(render(&mut filtered, &[])) / energy(render(&mut dry, &[]))).sqrt()
    }

    for (num, den) in [(24_000_000, 544), (32_000, 1), (96_000, 1)] {
        let tuning = Tuning::new(num, den).expect("audio clock");
        // A4 at the cutoff, then well inside the passband.
        assert!((0.69..0.72).contains(&gain(&tuning, 69, 440)));
        assert!(gain(&tuning, 69, 5000) > 0.99);
        // Doubling the frequency deep in the stopband costs about 24 dB.
        let octave = gain(&tuning, 93, 110) / gain(&tuning, 81, 110);
        assert!((0.05..0.09).contains(&octave));
    }
}

#[test]
fn cutoff_changes_on_its_frame_and_continues_across_blocks() {
    let tuning = Tuning::new(48_000, 1).expect("audio clock");
    let mut whole = Monosynth::new(&tuning);
    let mut split = whole.clone();
    let start = [
        at(0, Message::Waveform(Waveform::Sawtooth)),
        at(
            0,
            Message::NoteOn {
                note: 69,
                velocity: 120,
            },
        ),
    ];
    let cutoff = Message::CutoffHz(110);
    let block = render::<256>(&mut whole, &[start[0], start[1], at(64, cutoff)]);
    assert_eq!(block[..64], render::<64>(&mut split, &start));
    let mut dry = split.clone();
    assert_eq!(block[64..144], render::<80>(&mut split, &[at(0, cutoff)]));
    assert_eq!(block[144..], render::<112>(&mut split, &[]));
    assert_ne!(block[64..], render::<192>(&mut dry, &[]));
    assert_eq!(
        render::<16>(&mut split, &[at(0, Message::NoteOff(69))]),
        [0; 16]
    );
}

#[test]
fn a_lookahead_voice_starts_at_the_live_phase_and_advances_independently() {
    let tuning = Tuning::new(1760, 1).expect("four frames per A4 cycle");
    let mut live = Monosynth::new(&tuning);
    render::<1>(
        &mut live,
        &[at(
            0,
            Message::NoteOn {
                note: 69,
                velocity: 8,
            },
        )],
    );
    let mut lookahead = live.clone();
    assert_eq!(render::<2>(&mut lookahead, &[]), [32767, 0]);
    assert_eq!(render::<2>(&mut live, &[]), [32767, 0]);
    assert_eq!(
        render::<1>(&mut lookahead, &[at(0, Message::NoteOff(69))]),
        [0]
    );
    assert_eq!(render::<1>(&mut live, &[]), [-32767]);
}

#[test]
fn all_presets_match_the_original_waveform_at_quarter_cycles() {
    let tuning = Tuning::new(3520, 2).expect("four frames per A4 cycle");
    for (wave, expected) in [
        (Waveform::Sine, [0, 32767, 0, -32767]),
        (Waveform::Sawtooth, [-32768, -16384, 0, 16384]),
        (Waveform::Square, [32767, 32767, -32767, -32767]),
        (Waveform::Triangle, [-32768, 0, 32766, -2]),
    ] {
        let mut voice = Monosynth::new(&tuning);
        let events = [
            at(0, Message::Waveform(wave)),
            at(
                0,
                Message::NoteOn {
                    note: 69,
                    velocity: 8,
                },
            ),
        ];
        assert_eq!(render::<4>(&mut voice, &events), expected);
    }
}

#[test]
fn note_changes_respect_offsets_and_releases_belong_to_the_current_note() {
    let tuning = Tuning::new(24_000_000, 544).expect("FM-1 audio clock");
    assert!(Tuning::new(0, 1).is_none());
    assert!(Tuning::new(44100, 0).is_none());
    let mut voice = Monosynth::new(&tuning);
    let events = [
        at(1, Message::Waveform(Waveform::Square)),
        at(
            1,
            Message::NoteOn {
                note: 69,
                velocity: 8,
            },
        ),
        at(
            2,
            Message::NoteOn {
                note: 71,
                velocity: 16,
            },
        ),
        at(2, Message::NoteOff(69)),
        at(
            3,
            Message::NoteOn {
                note: 71,
                velocity: 0,
            },
        ),
    ];
    assert_eq!(render::<4>(&mut voice, &events), [0, 32767, 65534, 0]);
}

#[test]
fn vca_follows_the_gate_across_legato_notes_and_releases_from_sustain() {
    let tuning = Tuning::new(2_000, 2).expect("one frame per millisecond");
    let mut voice = Monosynth::new(&tuning);
    let adsr = Adsr {
        attack_us: 4_000,
        decay_us: 4_000,
        sustain_percent: 50,
        release_us: 4_000,
    };
    let key = |note| Message::NoteOn { note, velocity: 8 };
    let events = [
        at(2, Message::VcaEnvelope(adsr)),
        at(2, Message::Waveform(Waveform::Square)),
        at(2, key(48)),
        at(4, key(50)), // The second key and the older key-up leave the gate high.
        at(5, Message::NoteOff(48)),
        at(
            12,
            Message::NoteOn {
                note: 50,
                velocity: 0,
            },
        ),
        at(13, Message::NoteOff(50)), // A repeated note-off cannot extend the release.
    ];
    assert_eq!(
        render::<18>(&mut voice, &events).map(i32::abs),
        [
            0, 0, 8191, 16383, 24575, 32767, 25599, 20479, 17407, 16383, 16383, 16383, 9215, 4095,
            1023, 0, 0, 0,
        ]
    );
}

#[test]
fn vca_edits_retime_the_active_stage_and_a_new_gate_attacks_from_the_tail() {
    let tuning = Tuning::new(1_000, 1).expect("one frame per millisecond");
    let mut voice = Monosynth::new(&tuning);
    let mut adsr = Adsr {
        attack_us: 8_000,
        decay_us: 4_000,
        sustain_percent: 50,
        release_us: 8_000,
    };
    let key = Message::NoteOn {
        note: 48,
        velocity: 8,
    };
    let first = [
        at(0, Message::Waveform(Waveform::Square)),
        at(0, Message::VcaEnvelope(adsr)),
        at(0, key),
    ];
    assert_eq!(render::<2>(&mut voice, &first).map(i32::abs), [4095, 8191]);
    adsr.attack_us = 4_000;
    assert_eq!(
        render::<5>(&mut voice, &[at(0, Message::VcaEnvelope(adsr))]).map(i32::abs),
        [16383, 24575, 32767, 25599, 20479]
    );
    adsr.decay_us = 8_000;
    assert_eq!(
        render::<4>(&mut voice, &[at(0, Message::VcaEnvelope(adsr))]).map(i32::abs),
        [18687, 17407, 16639, 16383]
    );
    adsr.sustain_percent = 25;
    assert_eq!(
        render::<4>(&mut voice, &[at(0, Message::VcaEnvelope(adsr))]).map(i32::abs),
        [12799, 10239, 8703, 8191]
    );
    assert_eq!(
        render::<2>(&mut voice, &[at(0, Message::NoteOff(48))]).map(i32::abs),
        [6271, 4607]
    );
    adsr.release_us = 4_000;
    let mut uninterrupted = voice.clone();
    let edit = [at(0, Message::VcaEnvelope(adsr))];
    assert_eq!(
        render::<4>(&mut uninterrupted, &edit).map(i32::abs),
        [2047, 511, 0, 0]
    );
    assert_eq!(render::<2>(&mut voice, &edit).map(i32::abs), [2047, 511]);
    assert_eq!(
        render::<4>(&mut voice, &[at(0, key)]).map(i32::abs),
        [8575, 16639, 24703, 32767]
    );
}

#[test]
fn early_key_up_releases_from_the_partial_attack_at_the_device_clock() {
    for (num, den, attack, release) in [(24_000_000, 544, 441, 176), (48_000, 1, 480, 192)] {
        let tuning = Tuning::new(num, den).expect("audio clock");
        let mut voice = Monosynth::new(&tuning);
        let off = attack / 2;
        let events = [
            at(0, Message::Waveform(Waveform::Square)),
            at(
                0,
                Message::VcaEnvelope(Adsr {
                    attack_us: 10_000,
                    release_us: 4_000,
                    ..Adsr::GATE
                }),
            ),
            at(
                0,
                Message::NoteOn {
                    note: 48,
                    velocity: 64,
                },
            ),
            at(off, Message::NoteOff(48)),
        ];
        let samples = render::<512>(&mut voice, &events).map(i32::abs);
        let peak = samples[off - 1];
        assert!((130_000..=131_068).contains(&peak));
        assert!(samples[off] < peak);
        assert!(samples[off + release - 2] > 0);
        assert!(samples[off + release - 1..].iter().all(|&s| s == 0));
    }
}
