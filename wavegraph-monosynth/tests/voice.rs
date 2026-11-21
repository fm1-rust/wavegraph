//! Check pitch, waveform levels and note handling at exact frame offsets.
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_monosynth::{Message, Monosynth, Tuning, Waveform};
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
