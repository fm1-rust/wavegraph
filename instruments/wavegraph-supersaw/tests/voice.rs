//! Exercise the public message inlet and measure the rendered oscillator.
use core::f64::consts::TAU;
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_sample::{FromSample, I24, Sample};
use wavegraph_supersaw::{Adsr, Message, Supersaw, Tuning};

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

fn render<S: FromSample<f32>>(
    voice: &mut Supersaw<'_>,
    output: &mut [S],
    events: &[Event<Message>],
) {
    voice.process(&mut ProcessContext {
        frames: output.len(),
        inputs: &[],
        outputs: &mut [output],
        events,
        emit: &mut Err,
    });
}

#[test]
fn frame_offsets_ties_and_silent_blocks_preserve_phase_in_both_formats() {
    let tuning = Tuning::new(24_000_000, 544).expect("FM-1 audio clock");
    let mut whole = Supersaw::new(&tuning);
    let mut split = whole.clone();
    let events = [
        at(17, on(69, 127)),
        at(53, Message::Detune(126)),
        at(53, Message::Mix(127)),
        at(58, Message::CutoffHz(440)),
        at(62, Message::Resonance(100)),
        at(70, on(76, 80)),
        at(70, Message::NoteOff(69)),
        at(120, on(76, 0)),
        at(320, Message::FilterEnvelope(Adsr::GATE)),
        at(400, on(64, 90)),
    ];
    let mut signal = [f32::NAN; 512];
    render(&mut whole, &mut signal, &events);
    assert_eq!(signal[..17], [0.0; 17]);
    assert!(signal[71..120].iter().any(|&s| s.abs() > 0.001));
    assert!(signal[210..400].iter().all(|&s| s == 0.0));
    assert!(signal[450..].iter().any(|&s| s.abs() > 0.001));

    let mut pcm = [I24::default(); 512];
    let mut first = 0;
    for event in events {
        render(&mut split, &mut pcm[first..event.frame], &[]);
        render::<I24>(&mut split, &mut [], &[at(0, event.message)]);
        first = event.frame;
    }
    render(&mut split, &mut pcm[first..], &[]);
    assert_eq!(pcm, signal.map(Sample::to_sample::<I24>));
}

#[test]
fn invalid_notes_and_controls_are_bounded_at_the_public_inlet() {
    let tuning = Tuning::new(8000, 1).expect("audio clock");
    let mut voice = Supersaw::new(&tuning);
    let mut expected = voice.clone();
    let mut a = [0.0_f32; 512];
    let mut b = a;
    render(
        &mut voice,
        &mut a,
        &[
            at(0, Message::Detune(255)),
            at(0, Message::Mix(255)),
            at(0, Message::Resonance(255)),
            at(0, Message::CutoffHz(u16::MAX)),
            at(0, on(69, 255)),
            at(100, on(255, 20)),
            at(100, on(70, 0)),
        ],
    );
    render(
        &mut expected,
        &mut b,
        &[
            at(0, Message::Detune(127)),
            at(0, Message::Mix(127)),
            at(0, Message::Resonance(127)),
            at(0, Message::CutoffHz(u16::MAX)),
            at(0, on(69, 127)),
        ],
    );
    assert_eq!(a, b);
    render(&mut voice, &mut a, &[at(0, on(127, 100))]); // above output Nyquist
    render(&mut expected, &mut b, &[]);
    assert_eq!(a, b); // An unplayable pitch leaves the other keys sounding.
}

#[test]
fn live_cutoff_resonance_and_filter_sustain_shape_the_held_spectrum() {
    // Exactly 100 frames per fundamental period, so the bins need no window.
    let tuning = Tuning::new(44_000, 1).expect("audio clock");
    let mut voice = Supersaw::new(&tuning);
    let closed = Adsr {
        sustain_percent: 0,
        ..Adsr::GATE
    };
    render(
        &mut voice,
        &mut [0.0_f32; 2048],
        &[
            at(0, Message::CutoffHz(440)),
            at(0, Message::Mix(0)),
            at(0, Message::Detune(0)),
            at(0, Message::FilterEnvelope(closed)),
            at(0, on(69, 127)),
        ],
    );
    let measure = |message| {
        let mut held = voice.clone();
        render(&mut held, &mut [0.0_f32; 2048], &[at(0, message)]);
        let mut samples = [0.0_f32; 1000];
        render(&mut held, &mut samples, &[]);
        [1.0, 4.0].map(|harmonic| {
            let mut re = 0.0;
            let mut im = 0.0;
            for (i, &s) in (0_u16..).zip(&samples) {
                let (sin, cos) = (TAU * harmonic * f64::from(i) / 100.0).sin_cos();
                re += f64::from(s) * cos;
                im += f64::from(s) * sin;
            }
            re.hypot(im)
        })
    };
    let [fundamental, fourth] = measure(Message::Resonance(0));
    let [peak, _] = measure(Message::Resonance(127));
    let [_, opened] = measure(Message::CutoffHz(1760));
    let [_, swept] = measure(Message::FilterEnvelope(Adsr::GATE));
    assert!(peak > fundamental * 5.0);
    assert!(opened > fourth * 8.0);
    assert!((swept / opened - 1.0).abs() < 0.01);
}

#[test]
fn amplitude_sustain_and_release_are_independent_of_the_filter() {
    let tuning = Tuning::new(44_000, 1).expect("audio clock");
    let mut full = Supersaw::new(&tuning);
    render(&mut full, &mut [0.0_f32; 8192], &[at(0, on(57, 110))]);
    let mut quiet = full.clone();
    let envelope = Adsr {
        sustain_percent: 25,
        release_us: 10_000,
        ..Adsr::GATE
    };
    let mut a = [0.0_f32; 1024];
    let mut b = a;
    render(&mut full, &mut a, &[]);
    render(&mut quiet, &mut b, &[at(0, Message::AmpEnvelope(envelope))]);
    for (full, reduced) in a[200..].iter().zip(&b[200..]) {
        assert!((full * 0.25 - reduced).abs() < 0.000_001);
    }
    render(&mut quiet, &mut b, &[at(10, Message::NoteOff(57))]);
    assert!(b[100..400].iter().any(|s| s.abs() > 0.0001));
    assert!(b[450..].iter().all(|&s| s == 0.0));
}

#[test]
fn a_long_filter_release_keeps_time_while_the_amp_is_silent() {
    let tuning = Tuning::new(44_000, 1).expect("audio clock");
    let mut whole = Supersaw::new(&tuning);
    let filter = Adsr {
        attack_us: 40_000,
        release_us: 100_000,
        ..Adsr::GATE
    };
    render(
        &mut whole,
        &mut [0.0_f32; 2048],
        &[
            at(0, Message::FilterEnvelope(filter)),
            at(0, Message::CutoffHz(330)),
            at(0, Message::Resonance(96)),
            at(0, on(57, 127)),
        ],
    );
    render::<f32>(&mut whole, &mut [], &[at(0, Message::NoteOff(57))]);
    let mut split = whole.clone();
    let mut tail = [0.0_f32; 1024];
    render(&mut whole, &mut tail, &[]);
    assert!(tail[88..].iter().all(|&s| s == 0.0));
    for chunk in tail.chunks_mut(31) {
        render(&mut split, chunk, &[]);
    }
    let mut expected = [0.0_f32; 256];
    let mut actual = expected;
    let strike = [at(0, on(60, 127))];
    render(&mut whole, &mut expected, &strike);
    render(&mut split, &mut actual, &strike);
    assert_eq!(actual, expected);
}

#[test]
fn full_detune_has_seven_pitches_and_mix_only_raises_the_sides() {
    const RATE: u16 = 44_100;
    const FRAMES: usize = RATE as usize;
    let tuning = Tuning::new(u32::from(RATE), 1).expect("audio clock");
    let record = |mix| {
        let mut voice = Supersaw::new(&tuning);
        let mut samples = vec![0.0_f32; FRAMES];
        render(
            &mut voice,
            &mut [0.0_f32; 4096],
            &[
                at(0, Message::Mix(mix)),
                at(0, Message::Detune(127)),
                at(0, on(69, 127)),
            ],
        );
        render(&mut voice, &mut samples, &[]);
        samples
    };
    let magnitude = |samples: &[f32], hz: f64| {
        let mut re = 0.0;
        let mut im = 0.0;
        for (i, &s) in (0..RATE).zip(samples) {
            let t = TAU * f64::from(i) / f64::from(RATE);
            let windowed = f64::from(s) * (1.0 - t.cos());
            let (sin, cos) = (t * hz).sin_cos();
            re += windowed * cos;
            im += windowed * sin;
        }
        2.0 * re.hypot(im) / f64::from(RATE)
    };
    let quiet = record(0);
    let loud = record(127);
    let center = magnitude(&quiet, 440.0);
    // The fixed center level, saw fundamental 2/π and -3 dB high-pass at A4.
    let expected = (25.0 / 512.0) * (2.0 / core::f64::consts::PI) / 2.0_f64.sqrt();
    assert!((center / expected - 1.0).abs() < 0.01);
    assert!((magnitude(&loud, 440.0) / center - 1.0).abs() < 0.01);
    for hz in [391.51, 412.25, 431.38, 448.62, 467.48, 487.41] {
        let peak = magnitude(&loud, hz);
        assert!(peak > center, "missing side saw at {hz} Hz");
        assert!((31.0..35.0).contains(&(peak / magnitude(&quiet, hz))));
    }
}

#[test]
fn seventh_chord_matches_four_independent_stacks_and_releases_by_root() {
    use wavegraph_core::polyphony::{Chord, VoiceMode};
    let tuning = Tuning::new(44_118, 1).expect("clock");
    let mut chord = Supersaw::new(&tuning);
    let mut poly = chord.clone();
    let mut manual = [0.0_f32; 256];
    let mut actual = manual;
    let pitches = [60, 64, 67, 71];
    render(
        &mut chord,
        &mut actual,
        &[
            at(
                0,
                Message::VoiceMode(VoiceMode::Chord(Chord::MAJOR.turn(2))),
            ),
            at(0, on(60, 95)),
        ],
    );
    render(&mut poly, &mut manual, &pitches.map(|n| at(0, on(n, 95))));
    assert_eq!(actual, manual);
    let mut sum = [0.0_f32; 256];
    for n in pitches {
        let mut voice = Supersaw::new(&tuning);
        render(&mut voice, &mut manual, &[at(0, on(n, 95))]);
        for (s, v) in sum.iter_mut().zip(manual) {
            *s += v;
        }
    }
    assert_eq!(actual, sum);
    render(&mut chord, &mut actual, &[at(0, on(60, 0))]);
    assert!(actual[100..].iter().all(|&s| s == 0.0));
}

#[test]
fn synchronized_voices_can_diverge_after_a_restrike_or_release() {
    let tuning = Tuning::new(24_000_000, 544).expect("clock");
    let pitches = [48, 55, 60, 67];
    let events = [
        at(0, on(48, 85)),
        at(0, on(55, 85)),
        at(0, on(60, 85)),
        at(0, on(67, 85)),
        at(31, on(55, 127)),
        at(64, Message::NoteOff(48)),
        at(83, Message::CutoffHz(820)),
        at(181, on(67, 40)),
        at(193, Message::FilterEnvelope(Adsr::GATE)),
        at(220, Message::NoteOff(60)),
    ];
    let mut actual = [0.0_f32; 320];
    let mut expected = [0.0_f32; 320];
    render(&mut Supersaw::new(&tuning), &mut actual, &events);
    for pitch in pitches {
        let solo_events: Vec<_> = events
            .into_iter()
            .filter(|event| match event.message {
                Message::NoteOn { note, .. } | Message::NoteOff(note) => note == pitch,
                _ => true,
            })
            .collect();
        let mut solo = [0.0_f32; 320];
        render(&mut Supersaw::new(&tuning), &mut solo, &solo_events);
        for (sum, sample) in expected.iter_mut().zip(solo) {
            *sum += sample;
        }
    }
    assert_eq!(actual, expected);
}

#[test]
fn unison_returns_to_held_root_and_stale_off_cannot_release_stolen_voice() {
    use wavegraph_supersaw::VoiceMode;
    let tuning = Tuning::new(44_118, 1).expect("clock");
    let mut synth = Supersaw::new(&tuning);
    let mut out = [0.0_f32; 256];
    render(
        &mut synth,
        &mut out,
        &[
            at(0, Message::VoiceMode(VoiceMode::Unison)),
            at(0, on(60, 110)),
            at(32, on(67, 90)),
            at(64, Message::NoteOff(67)),
        ],
    );
    assert!(out[190..].iter().any(|s| s.abs() > 0.001));
    render(&mut synth, &mut out, &[at(0, Message::NoteOff(60))]);
    assert!(out[100..].iter().all(|&s| s == 0.0));
    render(
        &mut synth,
        &mut out,
        &[at(0, Message::VoiceMode(VoiceMode::Poly))],
    );
    render(
        &mut synth,
        &mut out,
        &[60, 62, 64, 65, 67].map(|n| at(0, on(n, 100))),
    );
    let mut reference = synth.clone();
    let mut expected = [0.0_f32; 256];
    render(&mut synth, &mut out, &[at(0, Message::NoteOff(60))]);
    render(&mut reference, &mut expected, &[]);
    assert_eq!(out, expected);
}
