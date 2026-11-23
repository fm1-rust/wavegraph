//! Exercise the public message inlet and measure the rendered oscillator.
use core::f64::consts::TAU;
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_sample::{FromSample, I24, Sample};
use wavegraph_supersaw::{Message, Supersaw, Tuning};

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
        at(70, on(76, 80)),
        at(70, Message::NoteOff(69)),
        at(120, on(76, 0)),
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
            at(0, on(69, 127)),
        ],
    );
    assert_eq!(a, b);
    render(&mut voice, &mut a, &[at(0, on(127, 100))]); // above output Nyquist
    assert_eq!(a, [0.0; 512]);
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
