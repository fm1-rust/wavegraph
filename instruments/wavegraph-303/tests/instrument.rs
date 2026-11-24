//! Render the 303 through the patch interpreter and compare it to the native voice.
use wavegraph_303::{Message, Tb303, Voice, VoiceParams, Waveform};
use wavegraph_dsp::{Event, Port, Process, ProcessContext, bytecode::Program, dsp::Software};
use wavegraph_sample::{I24, Sample, types::i24};

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
fn oscillator_wraps_are_relative_to_the_block_across_timed_events() {
    // A4 advances by exactly 1/64 turn per sample at this rate.
    let mut bass = Tb303::new(28_160.0).with_idle_suspension();
    let mut render = |frames, events: &[Event<Message>]| {
        let mut output = [0.0_f32; 160];
        bass.process(&mut ProcessContext {
            frames,
            inputs: &[],
            outputs: &mut [&mut output[..frames]],
            events,
            emit: &mut Err,
        });
        bass.last_cycle_start()
    };
    assert_eq!(render(128, &[]), None);
    let events = [
        at(11, note(69, true, false)),
        at(81, Message::Resonance(1.0)),
        at(145, Message::CutoffHz(500.0)),
    ];
    assert_eq!(render(160, &events), Some(138));
    assert_eq!(render(40, &[]), None);
    assert_eq!(render(4, &[]), Some(2));
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
        at(29, Message::AttackSeconds(0.012)),
        at(29, Message::OverdriveDb(18.0)),
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
    params.attack_s = 0.012;
    params.overdrive_db = 18.0;
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
        Message::AttackSeconds(-0.01),
        Message::AttackSeconds(f32::NAN),
        Message::OverdriveDb(-1.0),
        Message::OverdriveDb(37.0),
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

#[test]
fn output_gain_preserves_hot_voice_shape_before_pcm_saturation() {
    const GAIN: f32 = 0.0625;
    const FRAMES: usize = 8192;
    let mut unity = Tb303::new(RATE);
    let mut quieter_float = Tb303::new(RATE).with_output_gain(GAIN);
    let mut quieter_pcm = Tb303::new(RATE).with_output_gain(GAIN);
    let params = VoiceParams {
        cutoff_hz: 1200.0,
        resonance: 0.94,
        overdrive_db: 28.0,
        accent: 0.9,
        ..VoiceParams::default()
    };
    unity.params = params;
    quieter_float.params = params;
    quieter_pcm.params = params;
    let events = [
        at(0, note(38, true, false)),
        at(4096, note(45, false, true)),
    ];
    let program = Program::<1>::decode(PROGRAM).expect("303 patch");
    let mut raw = [[0.0_f32; FRAMES]];
    let mut scaled = [[0.0_f32; FRAMES]];
    let mut pcm = [[I24::default(); FRAMES]];
    for (voice, output) in [(&mut unity, &mut raw), (&mut quieter_float, &mut scaled)] {
        let mut objects: [&mut dyn Process<Message, f32>; 1] = [voice];
        program
            .bind(&mut objects)
            .expect("float source")
            .render(&mut Software, output, &[&events], |_, event| Err(event))
            .expect("hot float phrase");
    }
    let mut objects: [&mut dyn Process<Message, I24>; 1] = [&mut quieter_pcm];
    program
        .bind(&mut objects)
        .expect("PCM source")
        .render(&mut Software, &mut pcm, &[&events], |_, event| Err(event))
        .expect("hot PCM phrase");

    assert!(raw[0].iter().any(|sample| sample.abs() > 1.0));
    let peak = raw[0]
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        scaled[0].iter().all(|sample| sample.abs() < 1.0),
        "raw peak {peak}"
    );
    for ((&raw, &scaled), &pcm) in raw[0].iter().zip(&scaled[0]).zip(&pcm[0]) {
        assert_eq!(scaled, raw * GAIN);
        assert_eq!(pcm, scaled.to_sample::<I24>());
    }
    // If gain were applied after clamping, every sample would remain at or below
    // GAIN. These overrange raw samples retain their shape in the attenuated PCM.
    assert!(
        pcm[0]
            .iter()
            .any(|sample| sample.to_sample::<f32>().abs() > GAIN)
    );
}

#[test]
fn output_gain_rejects_invalid_values() {
    for gain in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        assert!(std::panic::catch_unwind(|| Tb303::new(RATE).with_output_gain(gain)).is_err());
    }
}
