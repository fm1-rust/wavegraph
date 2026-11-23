//! Playback contracts observable from a patch.

use wavegraph_core::{Event, Process, ProcessContext};
use wavegraph_sample::FromSample;
use wavegraph_sampler::{Message, Sampler};

fn event(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn process<S: FromSample<i16>>(voice: &mut Sampler<'_>, out: &mut [S], events: &[Event<Message>]) {
    voice.process(&mut ProcessContext {
        frames: out.len(),
        inputs: &[],
        outputs: &mut [out],
        events,
        emit: &mut Err,
    });
}

#[test]
fn native_frames_survive_block_boundaries_and_eof_stays_silent() {
    let mut voice =
        Sampler::from_pcm16le(&[0x00, 0x80, 0x34, 0x12, 0xff, 0x7f]).expect("whole PCM frames");
    let mut out = [99_i16; 3];
    process(&mut voice, &mut out, &[event(2, Message::Trigger)]);
    assert_eq!(out, [0, 0, i16::MIN]);
    process(&mut voice, &mut out, &[]);
    assert_eq!(out, [0x1234, i16::MAX, 0]);
    process(&mut voice, &mut out, &[]);
    assert_eq!(out, [0; 3]);
}

#[test]
fn retrigger_stop_and_ties_take_effect_before_the_addressed_frame() {
    use Message::{Stop, Trigger};

    let mut voice = Sampler::from_pcm16le(&[1, 0, 2, 0, 3, 0]).expect("whole PCM frames");
    let mut out = [99_i16; 8];
    let events = [
        event(0, Trigger),
        event(2, Trigger),
        event(3, Trigger),
        event(3, Stop),
        event(5, Stop),
        event(5, Trigger),
    ];
    process(&mut voice, &mut out, &events);
    assert_eq!(out, [1, 2, 1, 0, 0, 1, 2, 3]);
}

#[test]
fn zero_frame_calls_deliver_events_without_consuming_pcm() {
    let mut voice = Sampler::from_pcm16le(&[0, 0x80, 0, 0x40]).expect("whole PCM frames");
    process::<f32>(&mut voice, &mut [], &[event(0, Message::Trigger)]);
    process::<f32>(&mut voice, &mut [], &[]);
    let mut out = [9.0_f32; 3];
    process(&mut voice, &mut out, &[]);
    assert_eq!(out, [-1.0, 0.5, 0.0]);
}

#[test]
fn empty_clips_are_silent_and_partial_frames_are_rejected() {
    assert!(Sampler::from_pcm16le(&[1, 0, 2]).is_none());
    let mut voice = Sampler::from_pcm16le(&[]).expect("whole PCM frames");
    let mut out = [99_u16; 2];
    process(&mut voice, &mut out, &[event(0, Message::Trigger)]);
    assert_eq!(out, [32768; 2]);
}
