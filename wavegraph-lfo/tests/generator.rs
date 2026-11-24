//! Exercise the graph outlet and frame clock without an audio device.
use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_lfo::{Lfo, Shape};

#[test]
fn outlet_samples_block_start_and_wraps_after_one_cycle() {
    for (shape, expected) in [
        (Shape::Sine, [0.0, 1.0, 0.0, -1.0]),
        (Shape::Triangle, [0.0, 1.0, 0.0, -1.0]),
        (Shape::Saw, [-1.0, -0.5, 0.0, 0.5]),
        (Shape::Square, [1.0, 1.0, -1.0, -1.0]),
    ] {
        let mut lfo = Lfo::new(8192.0).expect("audio clock");
        lfo.set_frequency(8.0);
        lfo.shape = shape;
        for value in expected.into_iter().chain([expected[0]]) {
            assert_eq!(lfo.tick(0), None);
            let mut out = None;
            lfo.process(&mut ProcessContext::<_, f32> {
                frames: 256,
                inputs: &[],
                outputs: &mut [],
                events: &[],
                emit: &mut |e| {
                    assert!(out.replace(e).is_none());
                    Ok(())
                },
            });
            assert_eq!(
                out,
                Some(Event {
                    frame: 0,
                    port: 0,
                    message: value
                })
            );
        }
    }
}

#[test]
fn changing_rate_and_skipping_frames_preserve_phase() {
    let mut lfo = Lfo::new(8192.0).expect("audio clock");
    lfo.shape = Shape::Saw;
    lfo.set_frequency(8.0);
    lfo.advance(256);
    lfo.set_frequency(16.0);
    assert_eq!(lfo.tick(128).expect("tick").message, -0.5);
    assert_eq!(lfo.tick(1).expect("tick").message, 0.0);
}
