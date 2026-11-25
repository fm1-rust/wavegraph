//! Exercise FM spectra, note allocation and sample-timed patch changes.
use wavegraph_core::{Event, Process, ProcessContext, polyphony::Chord, sine_q15};
use wavegraph_fm::{Fm, Message, Patch, Ratio, Shape, Tuning, VoiceMode};
use wavegraph_sample::{FromSample, I24, Sample};

fn at(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn on(note: u8, velocity: u8) -> Message {
    Message::NoteOn { note, velocity }
}

fn render<S: FromSample<f32>>(fm: &mut Fm<'_>, out: &mut [S], events: &[Event<Message>]) {
    fm.process(&mut ProcessContext {
        frames: out.len(),
        inputs: &[],
        outputs: &mut [out],
        events,
        emit: &mut Err,
    });
}

const GATED: Patch = Patch {
    carrier: Shape::new(0, 0, 100, 0),
    pitch_cents: 0,
    ..Patch::TINE
};

#[test]
#[allow(
    clippy::cast_precision_loss,
    reason = "Sine table samples are signed 16-bit."
)]
fn zero_index_is_a_tuned_sine_and_out_of_band_modulation_falls_back_to_it() {
    let tuning = Tuning::new(8_000, 1).expect("clock");
    let note = 81; // 880 Hz: the seventh harmonic is above this clock's Nyquist.
    for (index, ratio) in [(0, Ratio::RootTwo), (127, Ratio::Seven)] {
        let mut out = [f32::NAN; 512];
        render(
            &mut Fm::new(&tuning),
            &mut out,
            &[
                at(
                    0,
                    Message::Patch(Patch {
                        index,
                        ratio,
                        feedback: 127,
                        ..GATED
                    }),
                ),
                at(0, on(127, 127)), // unplayable carrier at this rate
                at(8, on(note, 255)),
                at(320, on(note, 0)),
            ],
        );
        let mut phase: u32 = 0;
        assert_eq!(out[..8], [0.0; 8]);
        for &sample in &out[8..320] {
            assert_eq!(sample, sine_q15(phase) as f32 / (32768.0 * 4.0));
            phase = phase.wrapping_add(tuning.phase_step(note));
        }
        assert!(out[320..].iter().all(|&s| s == 0.0));
    }
}

#[test]
fn timed_edits_and_release_tails_survive_block_boundaries_in_pcm() {
    let tuning = Tuning::new(24_000_000, 544).expect("FM-1 audio clock");
    let mut whole = Fm::new(&tuning);
    let mut split = Fm::new(&tuning);
    let events = [
        at(13, on(55, 63)),
        at(87, on(62, 118)),
        at(191, Message::Patch(Patch::BELL)),
        at(191, on(65, 90)),
        at(440, Message::NoteOff(55)),
        at(581, on(62, 0)),
        at(700, Message::Patch(Patch::BASS)),
        at(711, Message::NoteOff(65)),
    ];
    let mut signal = [f32::NAN; 4096];
    render(&mut whole, &mut signal, &events);
    assert!(signal[..13].iter().all(|&s| s == 0.0));
    assert!(signal[711..900].iter().any(|&s| s.abs() > 0.001));
    assert!(signal[3600..].iter().all(|&s| s == 0.0));
    let mut pcm = [I24::default(); 4096];
    let mut start = 0;
    for event in events {
        render(&mut split, &mut pcm[start..event.frame], &[]);
        render::<I24>(&mut split, &mut [], &[at(0, event.message)]);
        start = event.frame;
    }
    render(&mut split, &mut pcm[start..], &[]);
    assert_eq!(pcm, signal.map(Sample::to_sample::<I24>));
}

#[test]
fn harder_tine_strikes_have_more_harmonics_and_lose_them_over_time() {
    // Exactly 100 samples per A4. Remove the fundamental from each whole-cycle window
    // and compare the remaining energy as a fraction of the signal's total energy.
    let tuning = Tuning::new(44_000, 1).expect("clock");
    fn overtone_fraction(cycle: &[f32]) -> f64 {
        let mut phase = 0.0_f64;
        let (mut sine, mut cosine, mut energy) = (0.0, 0.0, 0.0);
        for &s in cycle {
            let x = f64::from(s);
            sine += x * phase.sin();
            cosine += x * phase.cos();
            energy += x * x;
            phase += core::f64::consts::TAU / 100.0;
        }
        1.0 - 2.0 * (sine * sine + cosine * cosine) / (100.0 * energy)
    }
    let strike = |velocity| {
        let mut out = vec![0.0_f32; 25_000];
        render(&mut Fm::new(&tuning), &mut out, &[at(0, on(69, velocity))]);
        (
            overtone_fraction(&out[400..500]),
            overtone_fraction(&out[24_000..24_100]),
        )
    };
    let (hard, tail) = strike(127);
    let (soft, _) = strike(36);
    assert!(hard > soft + 0.15, "hard {hard}, soft {soft}");
    assert!(hard > tail + 0.25, "strike {hard}, tail {tail}");
}

#[test]
fn chord_mode_renders_the_three_pitches_and_fifth_key_steals_the_oldest() {
    let tuning = Tuning::new(44_100, 1).expect("clock");
    let mut chord = [0.0_f32; 256];
    render(
        &mut Fm::new(&tuning),
        &mut chord,
        &[
            at(0, Message::Patch(GATED)),
            at(0, on(60, 100)),
            at(0, Message::VoiceMode(VoiceMode::Chord(Chord::MAJOR))),
        ],
    );
    let mut sum = [0.0_f32; 256];
    for note in [60, 64, 67] {
        let mut part = [0.0_f32; 256];
        render(
            &mut Fm::new(&tuning),
            &mut part,
            &[at(0, Message::Patch(GATED)), at(0, on(note, 100))],
        );
        for (total, voice) in sum.iter_mut().zip(part) {
            *total += voice;
        }
    }
    assert_eq!(sum, chord);
    let mut fm = Fm::new(&tuning);
    let mut out = [f32::NAN; 256];
    let mut events = vec![at(0, Message::Patch(GATED))];
    for note in 60..65 {
        events.push(at(0, on(note, 127)));
    }
    for note in 61..65 {
        events.push(at(128, Message::NoteOff(note)));
    }
    render(&mut fm, &mut out, &events);
    assert!(out[..128].iter().any(|&s| s.abs() > 0.001));
    assert!(out.iter().all(|&s| s.abs() <= 1.0));
    assert!(out[128..].iter().all(|&s| s == 0.0));
}

#[test]
fn pitch_peak_transposes_both_operators_together_in_either_direction() {
    let tuning = Tuning::new(44_100, 1).expect("clock");
    for (depth, transposed) in [(1200, 81), (-1200, 57)] {
        let patch = Patch {
            ratio: Ratio::RootTwo,
            feedback: 83,
            pitch: Shape::new(0, 0, 100, 0),
            ..GATED
        };
        let mut bent = [0.0_f32; 1024];
        let mut reference = bent;
        render(
            &mut Fm::new(&tuning),
            &mut bent,
            &[
                at(
                    0,
                    Message::Patch(Patch {
                        pitch_cents: depth,
                        ..patch
                    }),
                ),
                at(0, on(69, 113)),
            ],
        );
        render(
            &mut Fm::new(&tuning),
            &mut reference,
            &[at(0, Message::Patch(patch)), at(0, on(transposed, 113))],
        );
        for (actual, expected) in bent.into_iter().zip(reference) {
            assert!((actual - expected).abs() < 0.0002);
        }
    }
}

#[test]
fn staggered_notes_run_independent_operator_and_pitch_envelopes() {
    let tuning = Tuning::new(8000, 1).expect("clock");
    let patch = Message::Patch(Patch {
        carrier: Shape::new(8_000, 125_000, 65, 80_000),
        modulator: Shape::new(3_000, 40_000, 12, 32_000),
        pitch: Shape::new(16_000, 80_000, 30, 50_000),
        pitch_cents: -700,
        ..Patch::TINE
    });
    let mut chord = [0.0_f32; 2048];
    render(
        &mut Fm::new(&tuning),
        &mut chord,
        &[
            at(0, patch),
            at(0, on(50, 113)),
            at(211, on(57, 82)),
            at(600, Message::NoteOff(50)),
            at(1130, Message::NoteOff(57)),
        ],
    );
    let mut solo1 = [0.0_f32; 2048];
    let mut solo2 = solo1;
    render(
        &mut Fm::new(&tuning),
        &mut solo1,
        &[
            at(0, patch),
            at(0, on(50, 113)),
            at(600, Message::NoteOff(50)),
        ],
    );
    render(
        &mut Fm::new(&tuning),
        &mut solo2,
        &[
            at(0, patch),
            at(211, on(57, 82)),
            at(1130, Message::NoteOff(57)),
        ],
    );
    for ((both, first), second) in chord.into_iter().zip(solo1).zip(solo2) {
        assert_eq!(both, first + second);
    }
}

#[test]
fn muting_the_modulator_envelope_leaves_the_carrier_at_its_own_level() {
    let tuning = Tuning::new(44_100, 1).expect("clock");
    let mut actual = [0.0_f32; 512];
    let mut clean = actual;
    render(
        &mut Fm::new(&tuning),
        &mut actual,
        &[
            at(
                0,
                Message::Patch(Patch {
                    modulator: Shape::new(0, 0, 0, 0),
                    ..GATED
                }),
            ),
            at(0, on(69, 127)),
        ],
    );
    render(
        &mut Fm::new(&tuning),
        &mut clean,
        &[
            at(0, Message::Patch(Patch { index: 0, ..GATED })),
            at(0, on(69, 127)),
        ],
    );
    assert_eq!(actual, clean);
    assert!(actual.iter().any(|&s| s.abs() > 0.24));
}
