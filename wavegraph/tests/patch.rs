//! A signal cord and a message cord connect two gain objects.
use core::ops::Range;
use heapless::Vec;
use wavegraph::{Event, Port, Process, ProcessContext};
use wavegraph_sample::{I24, Sample};

struct Gain(f32);

impl<S: Sample<Float = f32>> Process<f32, S> for Gain {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal, Port::Message]
    }

    fn outlets(&self) -> &[Port] {
        &[Port::Message, Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, f32, S>) {
        let input = cx.inputs[0];
        let output = &mut *cx.outputs[0];
        let mut render = |span: Range<usize>, gain| {
            for (&sample, out) in input[span.clone()].iter().zip(&mut output[span]) {
                *out = sample.mul_amp(gain);
            }
        };

        let mut start = 0;
        for event in cx.events {
            assert_eq!(event.port, 1);
            render(start..event.frame, self.0);
            self.0 = event.message;
            (cx.emit)(Event { port: 0, ..*event }).expect("room for each gain change");
            start = event.frame;
        }
        render(start..cx.frames, self.0);
    }
}

fn render<S: Sample<Float = f32>>(input: [S; 4]) -> [S; 4] {
    let mut first = Gain(1.0);
    let mut second = Gain(1.0);
    let first: &mut dyn Process<f32, S> = &mut first;
    let second: &mut dyn Process<f32, S> = &mut second;
    assert_eq!(first.outlets()[1], second.inlets()[0]);
    assert_eq!(first.outlets()[0], second.inlets()[1]);

    let events = [(0, 0.5), (2, 0.0), (2, 1.0)].map(|(frame, message)| Event {
        frame,
        port: 1,
        message,
    });
    let mut inbox = Vec::<_, 3>::new();
    let mut bus = [S::EQUILIBRIUM; 4];
    first.process(&mut ProcessContext {
        frames: input.len(),
        inputs: &[&input],
        outputs: &mut [&mut bus],
        events: &events,
        emit: &mut |event| {
            assert_eq!(event.port, 0);
            inbox.push(Event { port: 1, ..event }).map_err(|_| event)
        },
    });
    let mut output = [S::EQUILIBRIUM; 4];
    second.process(&mut ProcessContext {
        frames: bus.len(),
        inputs: &[&bus],
        outputs: &mut [&mut output],
        events: &inbox,
        emit: &mut |_| Ok(()),
    });
    output
}

#[test]
fn connected_objects_share_signals_and_deliver_ordered_messages() {
    assert_eq!(render([0.5_f32; 4]), [0.125, 0.125, 0.5, 0.5]);
    let half = 0.5_f32.to_sample::<I24>();
    assert_eq!(
        render([half; 4]).map(I24::inner),
        [1 << 20, 1 << 20, 1 << 22, 1 << 22]
    );
}

#[test]
fn messages_can_arrive_while_audio_is_stopped() {
    let mut gain = Gain(1.0);
    let mut emitted = Vec::<_, 1>::new();
    let change = Event {
        frame: 0,
        port: 1,
        message: 0.25,
    };
    gain.process(&mut ProcessContext::<f32, f32> {
        frames: 0,
        inputs: &[&[]],
        outputs: &mut [&mut []],
        events: &[change],
        emit: &mut |event| emitted.push(event),
    });
    assert_eq!(gain.0, 0.25);
    assert_eq!(&*emitted, &[Event { port: 0, ..change }]);
}
