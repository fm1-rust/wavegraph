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
