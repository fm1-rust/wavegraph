//! Measure the master transfer curve, each EQ band and the limiter's recovery.
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_fx::{Dynamics, MasterBus, MasterControls};
use wavegraph_sample::I24;

const RATE: u32 = 48_000;
const RAIL: i32 = 1 << 23;

fn master() -> MasterBus {
    MasterBus::new(RATE).expect("audio clock")
}

#[test]
fn steady_compression_has_the_requested_db_ratio() {
    // A −6.02 dBFS tone is 17.98 dB above threshold. Allow 0.25 dB for curve interpolation.
    for ratio in [2, 3, 4, 8, 16] {
        let mut bus = master();
        bus.edit(MasterControls::Dynamics(Some(Dynamics {
            threshold_db: -24,
            ratio_x2: ratio,
            makeup_db: 0,
            ceiling_db: 0,
        })));
        let input = RAIL / 2;
        assert_eq!(bus.process_sample(input).inner(), input);
        let mut y = 0;
        for _ in 0..RATE / 2 {
            y = bus.process_sample(input).inner();
        }
        let output_db = 20.0 * (f64::from(y) / f64::from(RAIL)).log10();
        let expected_db = -24.0 + (24.0 - 20.0 * 2_f64.log10()) * 2.0 / f64::from(ratio);
        assert!(
            (output_db - expected_db).abs() < 0.25,
            "ratio {ratio}/2: {output_db} dB"
        );
    }
}

#[test]
fn limiter_reduces_following_samples_and_enforces_both_rails_after_makeup() {
    let mut bus = master();
    bus.edit(MasterControls::Dynamics(Some(Dynamics {
        threshold_db: 0,
        ratio_x2: 2,
        makeup_db: 6,
        ceiling_db: -6,
    })));
    let ceiling = f64::from(RAIL) * 10_f64.powf(-6.0 / 20.0);
    for peak in [RAIL * 3, -RAIL * 5] {
        let y = f64::from(bus.process_sample(peak).inner());
        assert!((y.abs() - ceiling).abs() < 2.0);
    }
    let quiet = RAIL / 16;
    let following = bus.process_sample(quiet).inner();
    assert!(following < quiet / 4);
    for _ in 0..RATE {
        let _ = bus.process_sample(quiet);
    }
    let recovered = bus.process_sample(quiet).inner();
    assert!((f64::from(recovered) / f64::from(quiet) - 10_f64.powf(6.0 / 20.0)).abs() < 0.005);
    assert!(bus.dynamics_gain() > 0.999);
}

#[test]
fn eq_boosts_each_band_in_its_own_frequency_range() {
    fn energy(freq: f64, bands: [i8; 3]) -> f64 {
        let mut bus = master();
        bus.edit(MasterControls::Eq(Some(bands)));
        let mut sum = 0.0;
        for i in 0..4_800 {
            let phase = core::f64::consts::TAU * freq * f64::from(i) / f64::from(RATE);
            #[allow(clippy::cast_possible_truncation, reason = "Test tone is within Q23")]
            let x = (phase.sin() * 100_000.0) as i32;
            let y = f64::from(bus.process_sample(x).inner());
            if i >= 2400 {
                sum += y * y;
            }
        }
        sum
    }
    for (freq, band) in [(40.0, 0), (900.0, 1), (12_000.0, 2)] {
        let flat = energy(freq, [0; 3]);
        for i in 0..3 {
            let mut eq = [0; 3];
            eq[i] = 6;
            let db = 10.0 * (energy(freq, eq) / flat).log10();
            if i == band {
                assert!(db > 4.0, "{freq} Hz, band {i}: {db}");
            } else {
                assert!(db < 3.0, "{freq} Hz, band {i}: {db}");
            }
        }
    }
}

#[test]
fn bypass_and_frame_stamped_edits_work_for_the_graph_node() {
    assert!(MasterBus::new(0).is_none());
    let mut bus = master();
    let x = [I24::new_unchecked(RAIL / 2); 4];
    let mut y = [I24::default(); 4];
    let event = |frame, d| Event {
        frame,
        port: 0,
        message: MasterControls::Dynamics(d),
    };
    bus.process(&mut ProcessContext {
        frames: 4,
        inputs: &[&x],
        outputs: &mut [&mut y],
        events: &[
            event(
                1,
                Some(Dynamics {
                    ceiling_db: -12,
                    ..Dynamics::GLUE
                }),
            ),
            event(3, None),
        ],
        emit: &mut Err,
    });
    assert_eq!((y[0], y[3]), (x[0], x[3]));
    assert!(y[1].inner() < x[1].inner() && y[2] == y[1]);
}

#[test]
fn silent_spans_match_scalar_state_through_decay_and_new_attacks() {
    let mut scalar = master();
    scalar.edit(MasterControls::Dynamics(Some(Dynamics::GLUE)));
    scalar.edit(MasterControls::Eq(Some([12, -3, 5])));
    let mut spans = scalar;
    for step in 0..2200 {
        let len = [0, 1, 15, 16, 17, 127, 128, 129][step % 8];
        if step == 13 || step == 2003 {
            for input in [RAIL - 1, -RAIL, RAIL / 2] {
                assert_eq!(spans.process_sample(input), scalar.process_sample(input));
            }
        }
        let mut silent = [I24::default(); 129];
        spans.process_silence(&mut silent[..len]);
        for &out in &silent[..len] {
            assert_eq!(out, scalar.process_sample(0));
        }
        assert_eq!(spans, scalar);
    }
}
