//! Response and sample-timed retriggering of the pattern-controlled filter.

use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_fx::{FILTER_PATTERNS, FilterMode, Message, PatternFilter, PatternFilterControls};
use wavegraph_sample::I24;

const LEVEL: i32 = 1 << 18;

// A 3 kHz tone at 48 kHz, well above the default resting cutoff.
const TONE: [i32; 16] = [
    0, 100318, 185364, 242189, LEVEL, 242189, 185364, 100318, 0, -100318, -185364, -242189, -LEVEL,
    -242189, -185364, -100318,
];

#[test]
fn accents_open_the_filter_on_a_sustained_tone_then_decay() {
    let mut resting = PatternFilter::new(48_000).expect("valid sample rate");
    for i in 0..2048 {
        resting.tick(I24::new_unchecked(TONE[i % 16]));
    }
    let mut ghost = resting;
    let mut accent = resting;
    ghost.trigger(48);
    accent.trigger(127);
    let mut peaks = [0; 3];
    for i in 0..48_000 {
        let input = I24::new_unchecked(TONE[i % 16]);
        let sample = [&mut resting, &mut ghost, &mut accent].map(|fx| fx.tick(input).inner());
        if (128..512).contains(&i) {
            for (peak, out) in peaks.iter_mut().zip(sample) {
                *peak = (*peak).max(out.abs());
            }
        }
        if i >= 47_984 {
            assert!(
                sample[0].abs_diff(sample[2]) <= 2,
                "return to the resting cutoff"
            );
        }
    }
    assert!(
        peaks[1] > peaks[0] * 2 && peaks[2] > peaks[1] * 2,
        "{peaks:?}"
    );
}

#[test]
fn filter_modes_have_distinct_dc_response_and_zero_amount_stays_static() {
    assert!(PatternFilter::new(0).is_none());
    let dc = I24::new_unchecked(LEVEL);
    for (mode, expected) in [(FilterMode::LowPass, LEVEL), (FilterMode::BandPass, 0)] {
        let mut fx = PatternFilter::new(44_118).expect("valid sample rate");
        fx.set_controls(PatternFilterControls {
            amount: 0,
            mode,
            ..PatternFilterControls::INITIAL
        });
        let mut fixed = fx;
        for i in 0..4096 {
            if i % 37 == 0 {
                fx.trigger(127);
            }
            assert_eq!(fx.tick(dc), fixed.tick(dc));
        }
        assert!(fx.tick(dc).inner().abs_diff(expected) <= 3);
    }
}

#[test]
fn graph_events_act_at_their_sample_and_survive_block_splits() {
    let input: [_; 256] = core::array::from_fn(|i| I24::new_unchecked(TONE[i % 16]));
    let mut whole = [I24::default(); 256];
    let mut split = whole;
    let mut fx = PatternFilter::new(48_000).expect("valid sample rate");
    let mut divided = fx;
    let mut reference = fx;
    let event = |frame, message| Event {
        frame,
        port: 0,
        message,
    };
    let accent = Message::FilterTrigger(127);
    let bypass = Message::Enabled(false);
    let render =
        |fx: &mut PatternFilter, input: &[I24], output: &mut [I24], events: &[Event<Message>]| {
            fx.process(&mut ProcessContext {
                frames: input.len(),
                inputs: &[input],
                outputs: &mut [output],
                events,
                emit: &mut Err,
            });
        };
    render(
        &mut fx,
        &input,
        &mut whole,
        &[event(71, accent), event(160, bypass)],
    );
    render(
        &mut divided,
        &input[..91],
        &mut split[..91],
        &[event(71, accent)],
    );
    // A rest partway through the decay must leave its tail intact.
    render(
        &mut divided,
        &input[91..],
        &mut split[91..],
        &[event(0, Message::FilterTrigger(0)), event(69, bypass)],
    );
    assert_eq!(whole, split);
    let plain = input.map(|v| reference.tick(v));
    assert_eq!(whole[..71], plain[..71]);
    assert_ne!(whole[71..160], plain[71..160]);
    assert_eq!(whole[223..], input[223..]);
}

#[test]
fn factory_patterns_have_distinct_rhythms_and_usable_accents() {
    for (i, pattern) in FILTER_PATTERNS.iter().enumerate() {
        assert_eq!(pattern.accents.iter().max(), Some(&127));
        assert!(
            FILTER_PATTERNS[..i]
                .iter()
                .all(|other| other.accents != pattern.accents && other.name != pattern.name)
        );
    }
}

#[test]
fn a_hot_resonant_filter_settles_after_an_impulse_at_different_rates() {
    for rate in [8_000, 44_118, 96_000] {
        for mode in [FilterMode::LowPass, FilterMode::BandPass] {
            let mut fx = PatternFilter::new(rate).expect("valid sample rate");
            fx.set_controls(PatternFilterControls {
                mode,
                cutoff_hz: u16::MAX,
                resonance: 128,
                amount: 128,
                ..PatternFilterControls::INITIAL
            });
            for _ in 0..64 {
                fx.tick(I24::default());
            }
            fx.trigger(127);
            assert_ne!(fx.tick(I24::new_unchecked((1 << 23) - 1)), I24::default());
            for _ in 0..4096 {
                fx.tick(I24::default());
            }
            for i in 0..64 {
                let sample = fx.tick(I24::new_unchecked(if i % 2 == 0 { LEVEL } else { -LEVEL }));
                assert!(sample.inner().abs() < (1 << 22));
            }
            for _ in 0..4096 {
                fx.tick(I24::default());
            }
            assert_eq!(fx.tick(I24::default()), I24::default());
        }
    }
}
