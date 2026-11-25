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
