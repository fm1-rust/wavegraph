//! Regression coverage for the shared Hoover ensemble.
use wavegraph_fx::{Chorus, ChorusControls};
use wavegraph_sample::I24;

#[test]
fn factory_stereo_matches_the_original_hoover_chorus() {
    let mut fx = Chorus::new(1.0 / 44118.0);
    // Golden stereo output from wavegraph-hoover at 999512c3, including ring wraps.
    let mut hash = 0xcbf29ce484222325_u64;
    for i in 0..44118 {
        let input = if i % 113 < 47 { 0.3125 } else { -0.1875 };
        for sample in fx.stereo_frame(input) {
            hash = (hash ^ u64::from(sample.to_bits())).wrapping_mul(0x100000001b3);
        }
    }
    assert_eq!(hash, 0xe18ae3074f00abd6);
}

#[test]
fn editing_a_bypassed_insert_preserves_its_delay_tail() {
    let mut fx = Chorus::new(1.0 / 44118.0);
    let controls = ChorusControls {
        depth: 0,
        mix: 100,
        ..ChorusControls::INITIAL
    };
    fx.set_controls(ChorusControls {
        enabled: false,
        ..controls
    });
    let impulse = I24::new_unchecked(1 << 22);
    assert_eq!(fx.tick(impulse), impulse);
    fx.set_controls(controls);
    let echo = (1..256).find(|_| fx.tick(I24::default()).inner() != 0);
    assert_eq!(echo, Some(154));
}

#[test]
fn mono_spans_match_individual_frames_across_edits_and_ring_wraps() {
    for rate in [8000.0, 44118.0, 192000.0] {
        let mut block = Chorus::new(1.0 / rate);
        let mut reference = block;
        let input: [f32; 149] = core::array::from_fn(|i| if i % 7 < 4 { -0.75 } else { 0.5 });
        for (enabled, mix, depth) in [
            (true, 37, 64),
            (false, 81, 0),
            (true, 0, 127),
            (true, 100, 128),
        ] {
            let controls = ChorusControls {
                enabled,
                mix,
                depth,
                ..ChorusControls::INITIAL
            };
            block.set_controls(controls);
            reference.set_controls(controls);
            let mut out = [0.0_f32; 149];
            block.mono_block(&input, &mut out);
            for (x, actual) in input.into_iter().zip(out) {
                let [l, r] = reference.stereo_frame(x);
                assert_eq!(actual.to_bits(), ((l + r) * 0.5).to_bits());
            }
        }
    }
}

#[test]
fn silent_spans_preserve_the_tail_and_the_next_note() {
    let mut block = Chorus::new(1.0 / 48000.0);
    let mut reference = block;
    let mut out = [0.0_f32; 111];
    for i in 0..220 {
        if i == 110 {
            let controls = ChorusControls {
                rate: 47,
                tone: 0,
                ..ChorusControls::INITIAL
            };
            block.set_controls(controls);
            reference.set_controls(controls);
        }
        if [1, 111, 219].contains(&i) {
            let input = [0.7; 111];
            block.mono_block(&input, &mut out);
            for (x, actual) in input.into_iter().zip(out) {
                let [l, r] = reference.stereo_frame(x);
                assert_eq!(actual.to_bits(), ((l + r) * 0.5).to_bits());
            }
        } else {
            block.mono_silence(&mut out);
            for actual in out {
                let [l, r] = reference.stereo_frame(0.0);
                assert_eq!(actual.to_bits(), ((l + r) * 0.5).to_bits());
            }
        }
    }
}

#[test]
fn mks50_switch_keeps_dry_gain_and_the_wet_network_is_band_limited() {
    let rate = 44_118.0;
    let mut fx = Chorus::mks50(1.0 / rate);
    assert_eq!(fx.stereo_frame(0.5), [0.5; 2]);
    // Zero sweep holds the tap still. Measure the wet path on its own,
    // including insertion loss and fractional-tap interpolation.
    for (hz, low, high) in [
        (200.0, 0.86, 0.92),
        (1000.0, 0.58, 0.63),
        (2000.0, 0.35, 0.40),
        (4000.0, 0.19, 0.23),
        (8000.0, 0.11, 0.15),
        (12_000.0, 0.009, 0.025),
    ] {
        fx.set_controls(ChorusControls {
            depth: 0,
            mix: 100,
            ..ChorusControls::MKS50
        });
        let mut power = 0.0;
        for i in 0u16..10_000 {
            let phase = core::f32::consts::TAU * hz / rate * f32::from(i);
            let [l, r] = fx.stereo_frame(phase.sin() * 0.5);
            assert_eq!(l, r);
            if i >= 2_000 {
                power += l * l;
            }
        }
        let gain = (power / 1000.0).sqrt();
        assert!((low..high).contains(&gain), "{hz} Hz, gain {gain}");
    }
}

#[test]
fn mks50_stereo_spans_match_scalar_tails_and_safe_pcm_rails() {
    let mut block = Chorus::mks50(1.0 / 44_118.0);
    let mut reference = block;
    let input: [f32; 128] = core::array::from_fn(|i| if i % 99 < 75 { 0.98 } else { -1.0 });
    let mut mid = [I24::default(); 128];
    let mut side = mid;
    let mut moved = false;
    for span in 0..140 {
        let active = !(10..132).contains(&span);
        if active {
            block.mid_side_block(&input, &mut mid, &mut side);
        } else {
            block.mid_side_silence(&mut mid, &mut side);
        }
        for ((m, s), x) in mid.into_iter().zip(side).zip(input) {
            let [l, r] = reference.stereo_frame(if active { x } else { 0.0 });
            #[allow(
                clippy::cast_possible_truncation,
                reason = "Check the signed Q23 output bounds"
            )]
            let q = |v: f32| ((v * 8_388_608.0) as i32).clamp(-8_388_608, 8_388_607);
            assert_eq!(m.inner(), q((l + r) * 0.5));
            assert_eq!(s.inner(), q((l - r) * 0.5));
            if (100..132).contains(&span) {
                assert_eq!((m.inner(), s.inner()), (0, 0));
            }
            moved |= s.inner().abs() > 100_000;
        }
    }
    assert!(moved);
}
