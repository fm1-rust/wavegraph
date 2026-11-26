//! Whole-stream equivalence to the original public scalar processor, including
//! ramp interruption, control clamping, signed rounding and empty spans.

use wavegraph_fx::{Saturator, SaturatorControls};
use wavegraph_sample::{I24, types::i24};

const CONTROLS: [SaturatorControls; 7] = [
    SaturatorControls::INITIAL,
    SaturatorControls {
        drive: 0,
        mix: 128,
        level: 128,
        shape: 0,
        enabled: true,
    },
    SaturatorControls {
        drive: 255,
        mix: 255,
        level: 255,
        shape: 255,
        enabled: true,
    },
    SaturatorControls {
        drive: 128,
        mix: 127,
        level: 1,
        shape: 1,
        enabled: true,
    },
    SaturatorControls {
        drive: 64,
        mix: 1,
        level: 127,
        shape: 127,
        enabled: true,
    },
    SaturatorControls {
        drive: 0,
        mix: 0,
        level: 0,
        shape: 128,
        enabled: true,
    },
    SaturatorControls {
        drive: 255,
        mix: 128,
        level: 128,
        shape: 64,
        enabled: false,
    },
];

fn next(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

#[allow(
    clippy::cast_possible_wrap,
    reason = "Reinterpreting all signed bit patterns, then narrowing to Q23."
)]
fn samples(seed: &mut u32, count: usize) -> Vec<I24> {
    (0..count)
        .map(|i| match i % 9 {
            0 => i24::MIN,
            1 => i24::MAX,
            2 => I24::default(),
            3 => I24::new_unchecked(-1),
            4 => I24::new_unchecked(1),
            _ => I24::new_unchecked((next(seed) as i32) >> 8),
        })
        .collect()
}

fn compare(optimized: &mut Saturator, scalar: &mut Saturator, input: &[I24], partition: usize) {
    let expected: Vec<_> = input.iter().map(|&sample| scalar.tick(sample)).collect();
    let mut actual = input.to_vec();
    optimized.process_in_place(&mut []);
    for span in actual.chunks_mut(partition) {
        optimized.process_in_place(span);
    }
    assert_eq!(actual, expected);
}

#[test]
fn in_place_matches_scalar_at_every_ramp_interruption() {
    let mut seed = 0x7312_ba09;
    let input = samples(&mut seed, 257);
    for from in CONTROLS {
        for to in CONTROLS {
            for interrupt_at in 0..=64 {
                let mut optimized = Saturator::new();
                let mut scalar = optimized;
                optimized.set_controls(from);
                scalar.set_controls(from);
                compare(&mut optimized, &mut scalar, &input[..128], 128);
                optimized.set_controls(to);
                scalar.set_controls(to);
                compare(&mut optimized, &mut scalar, &input[..interrupt_at], 257);
                optimized.set_controls(from);
                scalar.set_controls(from);
                compare(&mut optimized, &mut scalar, &input, 17);
                // Re-enable both curves after a dry interval to expose any
                // hidden difference in the shape ramp's retained state.
                let probe = SaturatorControls {
                    drive: 17,
                    mix: 128,
                    level: 117,
                    shape: 73,
                    enabled: true,
                };
                optimized.set_controls(probe);
                scalar.set_controls(probe);
                compare(&mut optimized, &mut scalar, &input[..129], 63);
            }
        }
    }
}

#[test]
fn random_controls_and_partitions_preserve_the_complete_stream() {
    let mut optimized = Saturator::new();
    let mut scalar = optimized;
    let mut seed = 0x84ae_127f;
    const EDGES: [u8; 10] = [0, 1, 2, 63, 64, 65, 127, 128, 129, 255];
    const LENGTHS: [usize; 14] = [0, 1, 2, 3, 31, 32, 33, 63, 64, 65, 127, 128, 129, 257];
    const PARTITIONS: [usize; 8] = [1, 2, 3, 17, 31, 64, 128, 257];
    for episode in 0..1500 {
        if episode % 3 != 0 {
            let mut control =
                || EDGES[usize::try_from(next(&mut seed) % 10).expect("bounded palette index")];
            let controls = SaturatorControls {
                drive: control(),
                mix: control(),
                level: control(),
                shape: control(),
                enabled: next(&mut seed) & 1 != 0,
            };
            optimized.set_controls(controls);
            scalar.set_controls(controls);
        }
        let input = samples(&mut seed, LENGTHS[episode % LENGTHS.len()]);
        compare(
            &mut optimized,
            &mut scalar,
            &input,
            PARTITIONS[episode % PARTITIONS.len()],
        );
    }
}
