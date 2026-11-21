//! Render the 303 through the patch interpreter and compare it to the native voice.
use wavegraph::{Event, Port, Process, bytecode::Program, dsp::Software};
use wavegraph_303::{Message, Tb303, Voice, VoiceParams, Waveform};
use wavegraph_sample::{I24, types::i24};

const RATE: f32 = 24_000_000.0 / 544.0;
const PROGRAM: &[u8] = &[1, 1, 0, 0, 1]; // Call object 0 with one signal output.

fn at(frame: usize, message: Message) -> Event<Message> {
    Event {
        frame,
        port: 0,
        message,
    }
}

fn note(note: u8, accent: bool, slide: bool) -> Message {
    Message::NoteOn {
        note,
        accent,
        slide,
    }
}

#[test]
fn patch_matches_the_voice_across_notes_panel_changes_and_release() {
    let mut bass = Tb303::new(RATE);
    let program = Program::<1>::decode(PROGRAM).expect("303 patch");
    let mut objects: [&mut dyn Process<Message, f32>; 1] = [&mut bass];
    assert_eq!(objects[0].inlets(), [Port::Message]);
    assert_eq!(objects[0].outlets(), [Port::Signal]);
    let mut patch = program.bind(&mut objects).expect("mono source");
    let mut bus = [[f32::NAN; 97]];
    let events = [
        at(5, note(36, true, false)),
        at(29, Message::CutoffHz(850.0)),
        at(29, Message::Waveform(Waveform::Square)),
        at(53, note(43, false, true)),
        at(54, Message::NoteOff(36)),
        at(70, Message::NoteOff(43)),
    ];
    patch
        .render(&mut Software, &mut bus, &[&events], |_, e| Err(e))
        .expect("phrase");

    let mut voice = Voice::new(RATE);
    let mut params = VoiceParams::default();
    let mut expected = [0.0; 97];
    voice.render_add(&params, &mut expected[..5]);
    voice.note_on(36, true, false);
    voice.render_add(&params, &mut expected[5..29]);
    params.cutoff_hz = 850.0;
    params.square = true;
    voice.render_add(&params, &mut expected[29..53]);
    voice.note_on(43, false, true);
    voice.render_add(&params, &mut expected[53..70]);
    voice.note_off();
    voice.render_add(&params, &mut expected[70..]);
    assert_eq!(bus[0], expected);
    assert!(expected[70..].iter().any(|&s| s != 0.0));

    patch
        .render(&mut Software, &mut bus, &[], |_, e| Err(e))
        .expect("release tail");
    expected.fill(0.0);
    voice.render_add(&params, &mut expected);
    assert_eq!(bus[0], expected);
}

#[test]
fn controls_while_stopped_apply_in_order_without_advancing_the_voice() {
    let mut bass = Tb303::new(RATE);
    let program = Program::<1>::decode(PROGRAM).expect("303 patch");
    let mut objects: [&mut dyn Process<Message, f32>; 1] = [&mut bass];
    let mut patch = program.bind(&mut objects).expect("mono source");
    let events = [
        note(36, false, false),
        Message::Resonance(0.75),
        Message::EnvOctaves(2.5),
        Message::DecaySeconds(0.3),
        Message::Accent(0.9),
        Message::TuningSemitones(-0.5),
        Message::Reset,
        note(48, true, true), // Closed gate after reset triggers a fresh attack.
        note(128, false, false),
        Message::CutoffHz(f32::NAN),
        Message::Resonance(2.0),
    ]
    .map(|message| at(0, message));
    patch
        .render(&mut Software, &mut [[]], &[&events], |_, e| Err(e))
        .expect("zero frames");
    let mut bus = [[f32::NAN; 129]];
    patch
        .render(&mut Software, &mut bus, &[], |_, e| Err(e))
        .expect("first audio");

    let mut voice = Voice::new(RATE);
    let params = VoiceParams {
        resonance: 0.75,
        env_oct: 2.5,
        decay_s: 0.3,
        accent: 0.9,
        tuning_st: -0.5,
        ..Default::default()
    };
    let mut expected = [0.0; 129];
    voice.note_on(48, true, false);
    voice.render_add(&params, &mut expected);
    assert_eq!(bus[0], expected);
}

#[test]
fn pcm_output_saturates_at_both_rails() {
    let mut bass = Tb303::new(RATE);
    // Exaggerate the resonance makeup to exercise both rails in one note.
    bass.params.bass_makeup = 50.0;
    let params = bass.params;
    let program = Program::<1>::decode(PROGRAM).expect("303 patch");
    let mut objects: [&mut dyn Process<Message, I24>; 1] = [&mut bass];
    let mut patch = program.bind(&mut objects).expect("PCM source");
    let mut bus = [[i24::MAX; 1024]];
    patch
        .render(
            &mut Software,
            &mut bus,
            &[&[at(3, note(43, true, false))]],
            |_, e| Err(e),
        )
        .expect("PCM audio");

    let mut voice = Voice::new(RATE);
    let mut expected = [0.0; 1024];
    voice.render_add(&params, &mut expected[..3]);
    voice.note_on(43, true, false);
    voice.render_add(&params, &mut expected[3..]);
    assert!(expected.iter().any(|&x| x > 1.0));
    assert!(expected.iter().any(|&x| x < -1.0));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "PCM quantization truncates toward zero."
    )]
    for (&pcm, x) in bus[0].iter().zip(expected) {
        let quantized = (x * 8_388_608.0) as i32;
        assert_eq!(
            pcm.inner(),
            quantized.clamp(i24::MIN.inner(), i24::MAX.inner())
        );
    }
}
