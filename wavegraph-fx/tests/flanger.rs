//! Impulse response, sweep timing and sample-offset controls for the mono flanger.

use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_fx::{Flanger, FlangerControls, Message};
use wavegraph_sample::I24;

const PULSE: i32 = 1 << 20;

#[test]
fn half_wet_impulse_splits_across_fractional_taps_at_ring_wrap() {
    assert!(Flanger::new(0).is_none());
    let mut fx = Flanger::new(8_000).expect("sample rate");
    fx.set_controls(FlangerControls {
        depth: 0,
        feedback: 0,
        mix: 64,
        ..FlangerControls::INITIAL
    });
    // The 8.5-sample center tap reads these two impulses across the ring boundary.
    for _ in 0..63 {
        fx.tick(I24::default());
    }
    for (i, dry) in [PULSE, -PULSE].into_iter().chain([0; 16]).enumerate() {
        let expected = match i {
            0 => PULSE / 2,
            1 => -PULSE / 2,
            8 => PULSE / 4,
            10 => -PULSE / 4,
            _ => 0,
        };
        assert_eq!(fx.tick(I24::new_unchecked(dry)).inner(), expected);
    }
}

#[test]
fn lfo_rate_moves_the_comb_delay_in_real_time() {
    for (sample_rate, slow_delay) in [(8_000, 16), (16_000, 32)] {
        for (rate, expected_delay) in [(30, slow_delay), (63, 1)] {
            let mut fx = Flanger::new(sample_rate).expect("sample rate");
            fx.set_controls(FlangerControls {
                rate,
                depth: 128,
                feedback: 0,
                mix: 128,
                enabled: true,
            });
            // At 0.5 seconds the 1 Hz LFO is at the far tap, the 1.99 Hz LFO near the start.
            for _ in 0..sample_rate / 2 {
                fx.tick(I24::default());
            }
            let peak = (0..64)
                .map(|i| fx.tick(I24::new_unchecked(if i == 0 { PULSE } else { 0 })))
                .enumerate()
                .max_by_key(|(_, sample)| sample.inner())
                .expect("impulse response");
            assert_eq!(peak.0, expected_delay);
            assert!(peak.1.inner() > PULSE / 2);
        }
    }
}

#[test]
fn maximum_feedback_decays_to_silence_on_both_rails() {
    for rail in [-(1 << 23), (1 << 23) - 1] {
        let mut fx = Flanger::new(44_118).expect("sample rate");
        fx.set_controls(FlangerControls {
            depth: 0,
            feedback: u8::MAX,
            mix: u8::MAX,
            ..FlangerControls::INITIAL
        });
        for _ in 0..64 {
            fx.tick(I24::default());
        }
        // The center tap is exactly 32 frames. Follow its PCM16 feedback repeats.
        let mut echo = rail / 256;
        for i in 0..2048 {
            let out = fx.tick(I24::new_unchecked(if i == 0 { rail } else { 0 }));
            if i > 0 && i % 32 == 0 {
                assert_eq!(out.inner(), echo * 256);
                echo = echo * 3 / 4;
            } else {
                assert_eq!(out, I24::default());
            }
        }
        assert_eq!(echo, 0);
    }
}

#[test]
fn graph_bypass_and_enable_apply_at_the_event_frame() {
    let controls = FlangerControls {
        mix: 128,
        ..FlangerControls::INITIAL
    };
    let mut node = Flanger::new(44_118).expect("sample rate");
    let mut reference = node;
    let input = core::array::from_fn::<_, 256, _>(|i| {
        I24::new_unchecked(if i % 5 == 0 { PULSE + 1 } else { -PULSE - 3 })
    });
    let mut output = [I24::default(); 256];
    let events = [
        (0, Message::Flanger(controls)),
        (73, Message::Enabled(false)),
        (171, Message::Enabled(true)),
    ]
    .map(|(frame, message)| Event {
        frame,
        port: 0,
        message,
    });
    node.process(&mut ProcessContext {
        frames: input.len(),
        inputs: &[&input],
        outputs: &mut [&mut output],
        events: &events,
        emit: &mut Err,
    });
    for (i, sample) in input.into_iter().enumerate() {
        if matches!(i, 0 | 73 | 171) {
            reference.set_controls(FlangerControls {
                enabled: i != 73,
                ..controls
            });
        }
        assert_eq!(output[i], reference.tick(sample));
    }
    assert_eq!(&output[136..171], &input[136..171]);
    assert_ne!(&output[171..], &input[171..]);
}
