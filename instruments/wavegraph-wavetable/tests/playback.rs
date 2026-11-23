//! Verify the independent pitch, frame and envelope clocks through graph events.

use wavegraph_core::{Event, Process, ProcessContext, Tuning};
use wavegraph_sample::{FromSample, I24};
use wavegraph_wavetable::{Adsr, BANKS, Bank, Message, Mode, Scan, Wavetable};

fn event(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn on(note: u8) -> Message {
    Message::NoteOn {
        note,
        velocity: 127,
    }
}

fn render<S: FromSample<f32>>(voice: &mut Wavetable<'_>, out: &mut [S], events: &[Event<Message>]) {
    voice.process(&mut ProcessContext {
        frames: out.len(),
        inputs: &[],
        outputs: &mut [out],
        events,
        emit: &mut Err,
    });
}

fn command(voice: &mut Wavetable<'_>, message: Message) {
    render::<f32>(voice, &mut [], &[event(0, message)]);
}

#[test]
fn prepared_banks_validate_geometry_and_cover_both_ends() {
    assert!(Bank::from_packed12(&[]).is_none());
    assert!(Bank::from_packed12(&[0; 815]).is_none());
    assert!(Bank::from_packed12(&vec![0; 816 * 257]).is_none());
    // Constant frames make the expected crossfade observable independently of pitch.
    let mut pcm = [0; 816 * 2];
    pcm[816..].as_chunks_mut::<3>().0.fill([0, 4, 64]); // two Q15 values of 16384
    let banks = [Bank::from_packed12(&pcm).expect("two frames")];
    let tuning = Tuning::new(48_000, 1).expect("clock");
    assert!(Wavetable::new(&tuning, &[]).is_none());
    let mut voice = Wavetable::new(&tuning, &banks).expect("catalog");
    command(&mut voice, Message::Envelope(Adsr::GATE));
    command(&mut voice, on(69));
    let position = |start, mode| {
        Message::Scan(Scan {
            start,
            mode,
            ..Scan::RESONANT
        })
    };
    let mut out = [0.0; 5];
    render(
        &mut voice,
        &mut out,
        &[
            event(0, position(0, Mode::Smooth)),
            event(1, position(64, Mode::Smooth)),
            event(2, position(64, Mode::Stepped)),
            event(3, position(255, Mode::Smooth)),
            event(4, Message::NoteOff(69)),
        ],
    );
    assert_eq!(out, [0.0, 0.125 * 129.0 / 256.0, 0.125, 0.125, 0.0]);
}

#[test]
fn scan_duration_follows_elapsed_time_across_pitches_and_sample_rates() {
    for rate in [1_000, 2_000] {
        let tuning = Tuning::new(rate, 1).expect("clock");
        for note in [48, 60] {
            let mut voice = Wavetable::new(&tuning, &BANKS).expect("catalog");
            let scan = Scan {
                mode: Mode::OneShot,
                start: 127,
                end: 0,
                travel_ms: 100,
            };
            command(&mut voice, Message::Scan(scan));
            command(&mut voice, on(note));
            let mut half = vec![0.0; usize::try_from(rate / 20).expect("short interval")];
            assert_eq!(voice.frame(), 63);
            render(&mut voice, &mut half, &[]);
            assert_eq!(voice.frame(), 31);
            command(
                &mut voice,
                Message::Scan(Scan {
                    travel_ms: 200,
                    ..scan
                }),
            );
            assert_eq!(voice.frame(), 31);
            for expected in [15, 0, 0] {
                render(&mut voice, &mut half, &[]);
                assert_eq!(voice.frame(), expected);
            }
            command(
                &mut voice,
                Message::Scan(Scan {
                    mode: Mode::PingPong,
                    ..scan
                }),
            );
            command(&mut voice, on(note));
            for expected in [31, 0, 31, 63, 31] {
                render(&mut voice, &mut half, &[]);
                assert_eq!(voice.frame(), expected);
            }
        }
    }
}

#[test]
fn staggered_notes_keep_independent_scans_across_dma_boundaries() {
    let tuning = Tuning::new(24_000_000, 544).expect("clock");
    let mut chord = Wavetable::new(&tuning, &BANKS).expect("catalog");
    let mut early = Wavetable::new(&tuning, &BANKS).expect("catalog");
    let mut late = Wavetable::new(&tuning, &BANKS).expect("catalog");
    for voice in [&mut chord, &mut early, &mut late] {
        command(voice, Message::Bank(1));
        command(
            voice,
            Message::Scan(Scan {
                travel_ms: 10,
                ..Scan::VOCAL
            }),
        );
        command(voice, Message::Envelope(Adsr::GATE));
    }
    let mut entire = [0.0; 512];
    render(
        &mut chord,
        &mut entire,
        &[
            event(7, on(57)),
            event(131, on(69)),
            event(355, Message::NoteOff(57)),
        ],
    );
    let mut a = [0.0; 512];
    let mut b = a;
    for (block, (a, b)) in a
        .as_chunks_mut::<128>()
        .0
        .iter_mut()
        .zip(b.as_chunks_mut::<128>().0)
        .enumerate()
    {
        match block {
            0 => render(&mut early, a, &[event(7, on(57))]),
            2 => render(
                &mut early,
                a,
                &[event(
                    99,
                    Message::NoteOn {
                        note: 57,
                        velocity: 0,
                    },
                )],
            ),
            _ => render(&mut early, a, &[]),
        }
        if block == 1 {
            render(&mut late, b, &[event(3, on(69))]);
        } else {
            render(&mut late, b, &[]);
        }
    }
    assert!(b[135..].iter().any(|v: &f32| v.abs() > 0.01));
    for ((out, first), second) in entire.iter().zip(a).zip(b) {
        assert_eq!(*out, first + second);
    }
    assert_eq!(chord.frame(), late.frame());
}

#[test]
fn matching_off_retriggers_and_voice_stealing_on_i24() {
    let tuning = Tuning::new(44_100, 1).expect("clock");
    let mut v = Wavetable::new(&tuning, &BANKS).expect("catalog");
    command(&mut v, Message::Envelope(Adsr::GATE));
    for key in [52, 55, 59, 62, 67] {
        command(&mut v, on(key));
    }
    // This pitch was stolen. A stale off or invalid note must leave the four voices sounding.
    command(&mut v, Message::NoteOff(52));
    command(&mut v, on(200));
    command(&mut v, Message::Bank(99));
    let mut out = [I24::default(); 128];
    render(&mut v, &mut out, &[]);
    let mut expected = Wavetable::new(&tuning, &BANKS).expect("catalog");
    command(&mut expected, Message::Envelope(Adsr::GATE));
    for key in [55, 59, 62, 67] {
        command(&mut expected, on(key));
    }
    let mut four_voices = [I24::default(); 128];
    render(&mut expected, &mut four_voices, &[]);
    assert_eq!(out, four_voices);
    assert!(out.iter().any(|s| s.inner() != 0));
    for key in [55, 59, 62, 67] {
        command(&mut v, Message::NoteOff(key));
    }
    render(&mut v, &mut out, &[]);
    assert!(out.iter().all(|s| s.inner() == 0));
}
