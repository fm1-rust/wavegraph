//! Exercise a running program through the same entry points as a device.
use wavegraph::{
    Event, Op, Port, Process, ProcessContext,
    bytecode::{DecodeError, Error, Program},
    dsp::{Dsp, Software},
};
use wavegraph_monosynth::{Message, Monosynth, Tuning, Waveform};
use wavegraph_sample::{I24, types::i24};

const VOICE: &[u8] = &[1, 1, 0, 0, 1];

fn at(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

#[test]
fn live_edits_keep_oscillator_phase_and_reject_invalid_replacements() {
    // At four samples per A4 cycle, restarting the voice is unambiguous.
    let tuning = Tuning::new(1760, 1).expect("clock");
    let mut voice = Monosynth::new(&tuning);
    let mut parked = Monosynth::new(&tuning);
    let mut objects: [&mut dyn Process<Message, I24>; 2] = [&mut voice, &mut parked];
    let first = Program::<1>::decode(VOICE).expect("plain voice");
    let mut edit = [0; 8];
    edit[..VOICE.len()].copy_from_slice(VOICE);
    Op::Gain {
        input: 0,
        level: 64,
    }
    .encode(&mut edit[VOICE.len()..])
    .expect("host edit");
    let gain = Program::<2>::decode(&edit).expect("half gain");
    let incompatible = Program::<1>::decode(&[1, 1, 0, 0, 2]).expect("two outputs");
    let missing = Program::<1>::decode(&[1, 1, 2, 0, 1]).expect("unknown object");
    let mut patch = first.bind(&mut objects).expect("one voice");
    let notes = [
        at(0, Message::Waveform(Waveform::Sawtooth)),
        at(
            0,
            Message::NoteOn {
                note: 69,
                velocity: 8,
            },
        ),
    ];
    let mut buses = [[I24::default(); 2]; 2];
    patch
        .render(&mut Software, &mut buses, &[&notes], |_, e| Err(e))
        .expect("start");
    assert_eq!(buses[0].map(I24::inner), [-32768, -16384]);

    assert_eq!(patch.replace(&gain, 1), Err(Error::Buffers));
    patch.replace(&gain, buses.len()).expect("insert gain");
    assert_eq!(patch.replace(&incompatible, 2), Err(Error::Ports));
    assert_eq!(patch.replace(&missing, 2), Err(Error::Object));

    assert_eq!(
        patch.render(
            &mut Software,
            &mut buses,
            &[&[], &[at(0, Message::NoteOff(69))]],
            |_, e| Err(e)
        ),
        Err(Error::Event)
    );
    let invalid = [at(2, Message::NoteOff(69))];
    assert_eq!(
        patch.render(&mut Software, &mut buses, &[&invalid], |_, e| Err(e)),
        Err(Error::Event)
    );
    assert_eq!(
        patch.render(&mut Software, &mut buses[..1], &[], |_, e| Err(e)),
        Err(Error::Buffers)
    );
    patch
        .render(&mut Software, &mut buses, &[], |_, e| Err(e))
        .expect("next two frames");
    assert_eq!(buses[0].map(I24::inner), [0, 16384]);
    assert_eq!(buses[1].map(I24::inner), [0, 8192]);
}

#[test]
fn decode_checks_each_opcode_before_playback() {
    for (bytes, error) in [
        (&[][..], Error::Version),
        (&[2], Error::Version),
        (&[1, 4], Error::Instruction(DecodeError::UnknownOpcode(4))),
        (
            &[1, 1, 0, 2, 1, 0],
            Error::Instruction(DecodeError::Truncated),
        ),
        (&[1, 2, 0, 64], Error::Bus),
        (&[1, 1, 0, 1, 1, 0], Error::Bus),
        (&[1, 1, 0, 0, 1, 3, 0, 1], Error::Bus),
        (&[1, 1, 0, 0, 1, 1, 0, 0, 1], Error::Object),
        (&[1, 1, 0, 0, 3], Error::Capacity),
        (&[1, 1, 0, 0, 1, 2, 0, 64, 2, 1, 64], Error::Capacity),
    ] {
        assert_eq!(
            Program::<2, 2>::decode(bytes).expect_err("invalid bytecode"),
            error
        );
    }
}

// A control inlet appears between the signal inlets. It relays messages on
// outlet 1, between the two signal outlets.
struct Stereo;

impl Process<Message, f32> for Stereo {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal, Port::Message, Port::Signal]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal, Port::Message, Port::Signal]
    }
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, f32>) {
        cx.outputs[0].copy_from_slice(cx.inputs[0]);
        cx.outputs[1].copy_from_slice(cx.inputs[1]);
        for event in cx.events {
            (cx.emit)(*event).expect("room in host sink");
        }
    }
}

#[test]
fn native_multiport_objects_share_buses_and_emit_frame_offsets() {
    let program = Program::<4, 2>::decode(&[
        1, 1, 0, 0, 1, // voice -> bus 0
        2, 0, 64, // bus 0 at half gain -> bus 1
        1, 1, 2, 2, 1, 0, // stereo(bus 1, bus 0) -> buses 2, 3
        3, 2, 3, // mix buses 2 and 3 -> bus 4
    ])
    .expect("signal graph");
    let tuning = Tuning::new(44100, 1).expect("clock");
    let mut voice = Monosynth::new(&tuning);
    let mut stereo = Stereo;
    let mut objects: [&mut dyn Process<Message, f32>; 2] = [&mut voice, &mut stereo];
    let mut patch = program.bind(&mut objects).expect("matching objects");
    let notes = [
        at(1, Message::Waveform(Waveform::Square)),
        at(
            1,
            Message::NoteOn {
                note: 69,
                velocity: 64,
            },
        ),
    ];
    let relay = Event {
        port: 1,
        ..notes[0]
    };
    let mut emitted = None;
    let mut buses = [[0.0; 3]; 5];
    patch
        .render(&mut Software, &mut buses, &[&notes, &[relay]], |id, e| {
            emitted = Some((id, e));
            Ok(())
        })
        .expect("render");
    assert_eq!(emitted, Some((1, relay)));
    assert_eq!(buses[2], buses[1]);
    assert_eq!(buses[3], buses[0]);
    assert!(buses[4][1] > 0.0);
    assert_eq!(buses[4], buses[0].map(|s| s * 1.5));

    let mut stop = [[]; 5];
    patch
        .render(
            &mut Software,
            &mut stop,
            &[&[at(0, Message::NoteOff(69))]],
            |_, e| Err(e),
        )
        .expect("stopped event");
    patch
        .render(&mut Software, &mut buses, &[], |_, e| Err(e))
        .expect("silent");
    assert_eq!(buses[4], [0.0; 3]);
}

#[test]
fn software_dsp_saturates_both_rails() {
    let rails = [i24::MIN, i24::MAX];
    let mut output = [I24::default(); 2];
    Software.gain(&rails, 255, &mut output);
    assert_eq!(output, rails);
    Software.mix(&rails, &rails, &mut output);
    assert_eq!(output, rails);
    let mut floats = [0.0; 2];
    Software.mix(&[-1.0, 1.0], &[-0.75, 0.75], &mut floats);
    assert_eq!(floats, [-1.0, 1.0]);
}
