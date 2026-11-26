//! Post-VCA pedal controls, timing, silence, and independent output attenuation.

use wavegraph_303::{Destination, Message, Muffler, Tb303};
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_sample::{I24, Sample};

const RATE: f32 = 44_118.0;

fn at(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn note(number: u8) -> Message {
    Message::NoteOn {
        note: number,
        accent: true,
        slide: false,
    }
}

fn render(bass: &mut Tb303, events: &[Event<Message>], samples: &mut [f32]) {
    bass.process(&mut ProcessContext {
        frames: samples.len(),
        inputs: &[],
        outputs: &mut [samples],
        events,
        emit: &mut Err,
    });
}

#[test]
fn pedal_off_retains_the_legacy_filter_drive_bit_for_bit() {
    let mut dry = Tb303::new(RATE);
    let mut wet = Tb303::new(RATE);
    let controls = [at(0, Message::OverdriveDb(24.0)), at(0, note(38))];
    let mut a = [0.0; 4096];
    let mut b = [0.0; 4096];
    render(&mut dry, &controls, &mut a);
    render(&mut wet, &controls, &mut b);
    assert_eq!(a, b);

    render(&mut dry, &[], &mut a);
    render(&mut wet, &[at(0, Message::PedalDrive(0.75))], &mut b);
    assert_ne!(a, b);
    assert_eq!(wet.params.overdrive_db, 24.0);
    render(&mut dry, &[], &mut a);
    render(&mut wet, &[at(0, Message::PedalDrive(0.0))], &mut b);
    assert_eq!(a[64..], b[64..]);
    render(&mut dry, &[], &mut a);
    render(&mut wet, &[], &mut b);
    assert_eq!(a, b);
}

#[test]
fn pedal_changes_and_reset_do_not_create_audio_from_silence() {
    for suspend in [false, true] {
        let mut bass = Tb303::new(RATE);
        if suspend {
            bass = bass.with_idle_suspension();
        }
        let mut samples = [f32::NAN; 2048];
        for amount in [0.01, 1.0, 0.25, 0.0, 0.7] {
            render(
                &mut bass,
                &[at(0, Message::PedalDrive(amount))],
                &mut samples,
            );
            assert!(samples.iter().all(|&sample| sample == 0.0));
            render(&mut bass, &[at(0, Message::Reset)], &mut samples);
            assert!(samples.iter().all(|&sample| sample == 0.0));
            assert_eq!(bass.pedal_drive(), amount);
        }
    }
}

#[test]
fn timed_changes_and_ramps_are_independent_of_block_partitioning() {
    let events = [
        at(0, Message::OverdriveDb(18.0)),
        at(0, note(38)),
        at(17, Message::PedalDrive(0.7)),
        at(43, Message::Muffler(Muffler::Two)),
        at(61, Message::Muffler(Muffler::One)),
        at(79, Message::CutoffHz(1200.0)),
        at(129, Message::PedalDrive(0.2)),
        at(
            200,
            Message::NoteOn {
                note: 45,
                accent: false,
                slide: true,
            },
        ),
        at(277, Message::PedalDrive(0.95)),
        at(321, Message::Muffler(Muffler::Off)),
        at(399, Message::Modulate(Destination::PedalDrive, -0.2)),
        at(512, Message::Modulate(Destination::PedalDrive, 0.0)),
        at(600, Message::PedalDrive(0.0)),
        at(704, Message::Reset),
        at(704, note(50)),
        at(750, Message::PedalDrive(0.667)),
        at(920, Message::NoteOff(50)),
    ];
    let mut expected = [0.0; 1280];
    render(&mut Tb303::new(RATE), &events, &mut expected);

    let mut bass = Tb303::new(RATE);
    let mut actual = [0.0; 1280];
    let mut start = 0;
    for length in [1, 16, 3, 59, 50, 128, 64, 79, 112, 192, 216, 360] {
        let end = start + length;
        let local: Vec<_> = events
            .iter()
            .filter(|event| (start..end).contains(&event.frame))
            .map(|event| at(event.frame - start, event.message))
            .collect();
        render(&mut bass, &local, &mut actual[start..end]);
        start = end;
    }
    assert_eq!(start, actual.len());
    assert_eq!(actual, expected);
}

#[test]
fn muffler_feeds_both_taps_before_the_pedal_and_returns_to_exact_dry() {
    let mut original = Tb303::new(RATE).with_output_gain(0.0625);
    let mut bass = Tb303::new(RATE).with_output_gain(0.0625);
    let mut reference = [I24::default(); 4096];
    let mut spare = reference;
    let mut clean = reference;
    let mut driven = reference;
    for (mode, pedal) in [
        (Muffler::One, 0.0),
        (Muffler::Two, 0.7),
        (Muffler::Off, 0.0),
    ] {
        let controls = [
            at(0, Message::Reset),
            at(0, Message::OverdriveDb(12.0)),
            at(0, note(38)),
        ];
        original.process_with_clean_tap(&mut spare, &mut reference, &controls);
        bass.process_with_clean_tap(
            &mut [],
            &mut [],
            &[
                at(0, Message::Muffler(mode)),
                at(0, Message::PedalDrive(pedal)),
            ],
        );
        bass.process_with_clean_tap(&mut driven, &mut clean, &controls);
        assert_eq!(clean == reference, mode == Muffler::Off);
        assert_eq!(driven == clean, pedal == 0.0);
    }
}

#[test]
fn releasing_the_muffler_switch_restores_the_dry_voice_mid_note() {
    let mut bass = Tb303::new(RATE);
    let controls = [at(0, Message::OverdriveDb(18.0)), at(0, note(38))];
    let mut reference = [0.0; 4096];
    let mut actual = reference;
    render(&mut Tb303::new(RATE), &controls, &mut reference);
    render(
        &mut bass,
        &[
            controls[0],
            controls[1],
            at(100, Message::Muffler(Muffler::Two)),
            at(1800, Message::Muffler(Muffler::Off)),
        ],
        &mut actual,
    );
    assert_eq!(&actual[..100], &reference[..100]);
    assert_ne!(&actual[164..1800], &reference[164..1800]);
    assert_eq!(&actual[1864..], &reference[1864..]);
}

#[test]
fn output_headroom_does_not_change_pedal_character() {
    let events = [
        at(0, Message::OverdriveDb(24.0)),
        at(0, Message::PedalDrive(0.8)),
        at(0, note(43)),
    ];
    let mut unity = [0.0_f32; 4096];
    let mut quiet = [0.0_f32; 4096];
    let mut pcm = [I24::default(); 4096];
    render(&mut Tb303::new(RATE), &events, &mut unity);
    render(
        &mut Tb303::new(RATE).with_output_gain(0.0625),
        &events,
        &mut quiet,
    );
    Tb303::new(RATE)
        .with_output_gain(0.0625)
        .process(&mut ProcessContext {
            frames: pcm.len(),
            inputs: &[],
            outputs: &mut [&mut pcm],
            events: &events,
            emit: &mut Err,
        });
    assert!(unity.iter().any(|&sample| sample.abs() > 0.25));
    for ((raw, scaled), quantized) in unity.into_iter().zip(quiet).zip(pcm) {
        assert_eq!(scaled, raw * 0.0625);
        assert_eq!(quantized, scaled.to_sample::<I24>());
    }
}

#[test]
fn pedal_modulation_retains_both_stored_controls_and_invalid_values_are_ignored() {
    let mut bass = Tb303::new(RATE);
    let mut expected = Tb303::new(RATE);
    let controls = [
        at(0, Message::OverdriveDb(18.0)),
        at(0, Message::PedalDrive(0.5)),
        at(0, Message::Modulate(Destination::PedalDrive, 0.25)),
        at(0, note(48)),
    ];
    let mut actual = [0.0; 512];
    let mut reference = [0.0; 512];
    render(&mut bass, &controls, &mut actual);
    render(
        &mut expected,
        &[
            at(0, Message::OverdriveDb(18.0)),
            at(0, Message::PedalDrive(0.75)),
            at(0, note(48)),
        ],
        &mut reference,
    );
    assert_eq!(actual, reference);
    assert_eq!(bass.pedal_drive(), 0.5);
    assert_eq!(bass.params.overdrive_db, 18.0);
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        render(&mut bass, &[at(0, Message::PedalDrive(value))], &mut []);
        assert_eq!(bass.pedal_drive(), 0.5);
    }
}

#[test]
fn accepted_extremes_stay_finite_and_the_engaged_pedal_is_bounded() {
    for rate in [8000.0, RATE, 96_000.0] {
        for amount in [0.01, 0.5, 1.0] {
            for (cutoff, resonance, drive, key) in [(63.0, 0.0, 0.0, 0), (8000.0, 1.0, 36.0, 127)] {
                let mut bass = Tb303::new(rate);
                let mut samples = [0.0; 4096];
                render(
                    &mut bass,
                    &[
                        at(0, Message::CutoffHz(cutoff)),
                        at(0, Message::Resonance(resonance)),
                        at(0, Message::OverdriveDb(drive)),
                        at(0, Message::PedalDrive(amount)),
                        at(0, note(key)),
                    ],
                    &mut samples,
                );
                assert!(samples.iter().all(|sample| sample.is_finite()));
                assert!(samples[64..].iter().all(|sample| sample.abs() <= 4.49));
            }
        }
    }
}

#[test]
fn released_pedal_tails_settle_before_idle_suspension_and_reset_clears_them() {
    let mut bass = Tb303::new(RATE).with_idle_suspension();
    let mut samples = [0.0; 512];
    render(
        &mut bass,
        &[
            at(0, Message::OverdriveDb(18.0)),
            at(0, Message::Muffler(Muffler::Two)),
            at(0, Message::PedalDrive(1.0)),
            at(0, note(38)),
        ],
        &mut samples,
    );
    render(&mut bass, &[at(0, Message::NoteOff(38))], &mut samples);
    assert!(samples.iter().any(|&sample| sample != 0.0));
    for _ in 0..160 {
        render(&mut bass, &[], &mut samples);
    }
    assert!(samples.iter().all(|&sample| sample == 0.0));
    render(&mut bass, &[at(0, note(50))], &mut samples);
    assert!(samples.iter().any(|&sample| sample != 0.0));
    render(&mut bass, &[at(0, Message::Reset)], &mut samples);
    assert!(samples.iter().all(|&sample| sample == 0.0));
    assert_eq!(bass.pedal_drive(), 1.0);
}
