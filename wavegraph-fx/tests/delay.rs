//! Impulse response, delay-time changes and sample-offset control through the public node.

use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_fx::{Delay, Message};
use wavegraph_sample::I24;

fn at(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn render<const N: usize>(
    delay: &mut Delay<'_>,
    input: [i32; N],
    events: &[Event<Message>],
) -> [i32; N] {
    let mut output = [I24::default(); N];
    delay.process(&mut ProcessContext {
        frames: N,
        inputs: &[&input.map(I24::new_unchecked)],
        outputs: &mut [&mut output],
        events,
        emit: &mut |e| Err(e),
    });
    output.map(I24::inner)
}

#[test]
fn wet_echoes_obey_the_clock_and_keep_decaying_behind_a_muted_return() {
    let mut line = [123; 11];
    let mut delay = Delay::new(&mut line, 2000).expect("nonempty line");
    render(
        &mut delay,
        [],
        &[
            at(0, Message::TimeMs(3)),
            at(0, Message::Feedback(50)),
            at(0, Message::Wet(128)),
        ],
    );
    let x = 1 << 20;
    assert_eq!(render(&mut delay, [x, -x, 0, 0, 0], &[]), [0; 5]);
    // Global frames 6 and 7: mute only the second echo sample.
    assert_eq!(
        render(&mut delay, [0; 5], &[at(2, Message::Enabled(false))]),
        [0, x, 0, 0, 0]
    );
    // The second pass is half amplitude, including the previously muted sample.
    assert_eq!(
        render(&mut delay, [0; 5], &[at(2, Message::Enabled(true))]),
        [0, 0, x / 2, -x / 2, 0]
    );
    assert_eq!(
        render(&mut delay, [0; 5], &[at(3, Message::Wet(64))]),
        [0, 0, 0, x / 8, -x / 8]
    );
}

#[test]
fn delay_bounds_feedback_and_saturation_are_well_defined() {
    assert!(Delay::new(&mut [], 48_000).is_none());
    assert!(Delay::new(&mut [0], 0).is_none());
    for (ms, expected) in [(0, 1), (u16::MAX, 4)] {
        let mut line = [0; 4];
        let mut delay = Delay::new(&mut line, 1000).expect("nonempty line");
        let mut input = [0; 12];
        input[0] = 1 << 20;
        let mut echoes = [0; 12];
        echoes[expected] = input[0];
        assert_eq!(
            render(
                &mut delay,
                input,
                &[
                    at(0, Message::TimeMs(ms)),
                    at(0, Message::Feedback(0)),
                    at(0, Message::Wet(255))
                ]
            ),
            echoes
        );
    }
    let mut line = [0];
    let mut delay = Delay::new(&mut line, 44_118).expect("one-frame line");
    let full = -(1 << 23);
    assert_eq!(
        render(
            &mut delay,
            [full, full, 0, 0],
            &[at(0, Message::Feedback(255)), at(0, Message::Wet(128))]
        ),
        [0, full, full, (-32768 * 95 / 100) * 256]
    );
    render(&mut delay, [0; 256], &[]);
    assert_eq!(render(&mut delay, [0], &[]), [0]);
}

#[test]
fn moving_time_finishes_each_crossfade_and_picks_up_the_latest_edit() {
    let mut line = [0; 256];
    let mut delay = Delay::new(&mut line, 1000).expect("nonempty line");
    // A ramp makes the distance between read taps observable in the output.
    let ramp = |i: usize| i32::try_from(i).expect("small index") * 64 * 256;
    let warmup: [i32; 100] = core::array::from_fn(ramp);
    render(
        &mut delay,
        warmup,
        &[
            at(0, Message::TimeMs(2)),
            at(0, Message::Feedback(0)),
            at(0, Message::Wet(128)),
        ],
    );
    let input: [i32; 132] = core::array::from_fn(|i| ramp(100 + i));
    let out = render(
        &mut delay,
        input,
        &[
            at(0, Message::TimeMs(18)),
            at(16, Message::TimeMs(30)),
            at(32, Message::TimeMs(34)),
        ],
    );
    assert_eq!(out[0], ramp(98) - 16 * 256);
    assert_eq!(out[63], ramp(163 - 18));
    assert_eq!(out[127], ramp(227 - 34));
    assert_eq!(out[131], ramp(231 - 34));
    assert!(
        out.windows(2)
            .all(|pair| (pair[1] - pair[0]).abs() <= 64 * 256)
    );
}

#[test]
fn an_empty_delay_wakes_at_the_selected_time_after_silent_blocks() {
    let mut line = [0; 8];
    let mut delay = Delay::new(&mut line, 1000).expect("short line");
    let x = 1 << 20;
    render(
        &mut delay,
        [],
        &[at(0, Message::Feedback(0)), at(0, Message::Wet(128))],
    );
    assert_eq!(
        render(&mut delay, [0; 17], &[at(5, Message::TimeMs(2))]),
        [0; 17]
    );
    assert_eq!(render(&mut delay, [x, 0, 0, 0], &[]), [0, 0, x, 0]);
    assert_eq!(render(&mut delay, [0; 17], &[]), [0; 17]);
    assert_eq!(
        render(&mut delay, [x, 0, 0, 0], &[at(0, Message::TimeMs(3))]),
        [0, 0, 0, x]
    );
}

#[test]
fn beat_echo_keeps_the_factory_tap_and_reaches_four_beats_at_30_bpm() {
    use wavegraph_fx::{BeatClock, BeatDivision};
    let rate = (24_000_000, 544);
    for (beats, bpm) in [
        (BeatDivision::DOTTED_EIGHTH, 122),
        (BeatDivision::ONE, 120),
        (BeatDivision::ONE.turn(4), 30),
    ] {
        let samples =
            usize::try_from(beats.frames(bpm, rate).expect("clock")).expect("short duration");
        let mut native = vec![0; 16_544];
        let mut mid = vec![0; 23_162];
        let mut long = vec![0; 23_162];
        let mut delay = Delay::with_rate(&mut native, rate)
            .and_then(|d| d.with_long_taps(&mut mid, &mut long))
            .expect("echo storage");
        let mut actual = Vec::new();
        for block in 0..samples.div_ceil(128) + 2 {
            let mut input = [0; 128];
            let setup = [
                at(0, Message::BeatTime(BeatClock { beats, bpm })),
                at(0, Message::Wet(128)),
                at(0, Message::Feedback(0)),
            ];
            if block == 0 {
                input[..64].fill(1 << 21);
            }
            actual.extend(render(
                &mut delay,
                input,
                if block == 0 { &setup } else { &[] },
            ));
        }
        assert!(actual[..samples.saturating_sub(16)].iter().all(|&v| v == 0));
        assert!(
            actual[samples + 16..samples + 48]
                .iter()
                .all(|&v| v > (1 << 20))
        );
        assert!(actual[samples + 80..].iter().all(|&v| v == 0));
        if beats == BeatDivision::DOTTED_EIGHTH {
            assert_eq!(actual[samples], 1 << 21);
            assert_eq!(actual[samples - 1], 0);
        }
    }
}

#[test]
fn longest_tap_drains_across_single_frame_blocks() {
    fn response<const BLOCK: usize>() -> [i32; 128] {
        let (mut native, mut mid, mut long) = ([0; 16], [0; 12], [0; 5]);
        let mut delay = Delay::new(&mut native, 1000)
            .and_then(|d| d.with_long_taps(&mut mid, &mut long))
            .expect("three ascending tap lengths");
        render(
            &mut delay,
            [],
            &[
                at(0, Message::TimeMs(64)),
                at(0, Message::Feedback(0)),
                at(0, Message::Wet(128)),
            ],
        );
        let mut output = [0; 128];
        for (i, span) in output.as_chunks_mut::<BLOCK>().0.iter_mut().enumerate() {
            let mut input = [0; BLOCK];
            if i == 0 {
                input[0] = 1 << 21;
            }
            span.copy_from_slice(&render(&mut delay, input, &[]));
        }
        output
    }
    let reference = response::<128>();
    assert!(reference[65..80].iter().any(|&v| v != 0));
    assert_eq!(response::<1>(), reference);
}
