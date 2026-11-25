use super::*;
use crate::panel::PageButton;
use wavegraph_sampler::{Playback, Timbre};

fn first_frames(pcm: &[u8]) -> [i32; 128] {
    let frames = pcm.as_chunks::<2>().0;
    core::array::from_fn(|i| i32::from(i16::from_le_bytes(frames[i])) << 8)
}

#[test]
fn panel_and_midi_play_the_same_clip_an_octave_apart_on_independent_strips() -> Result<(), Error> {
    with_instrument(44_118, |engine| {
        let mut keys = Keyboard::<4>::new();
        for channel in [Channel::One, Channel::Two] {
            keys.nav.focus = channel;
            keys.nav.view = View::Scope;
            keys.nav.press(PageButton::Glo);
            keys.nav.turn(1, 0);
            let steps = (Part::Sampler as i8).saturating_sub(keys.nav.part() as i8);
            keys.nav.turn(0, steps);
            assert_eq!(keys.nav.part(), Part::Sampler);
            keys.tracks[channel.index()] =
                Track::factory(channel, keys.nav.assignments[channel.index()]);
        }
        keys.nav.focus = Channel::One;
        keys.nav.view = View::Scope;
        keys.press(note(60, 127));
        let (channel, action) = decode_midi([9, 0x91, 72, 127]).expect("channel two C5");
        keys.keys
            .midi(channel, action, &mut keys.tracks[channel.index()]);
        keys.snapshot();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        engine.render(keys.tracks, &mut buses)?;
        let root = buses[Channel::One.source_bus()].map(I24::inner);
        let octave = buses[Channel::Two.source_bus()].map(I24::inner);
        assert_eq!(
            octave,
            first_frames(include_bytes!("../../../samples/rave/horn.s16le"))
        );
        assert!(root.iter().any(|&x| x != 0));
        assert!(
            root.iter()
                .step_by(2)
                .copied()
                .eq(octave[..64].iter().copied())
        );
        assert!(buses[MASTER_BUS].iter().any(|s| s.inner() != 0));

        keys.tracks[0].turn_algorithm(View::Scope, None, 1);
        assert_eq!(keys.tracks[0].voice.label(Part::Sampler), b"LASER ZAP");
        assert_eq!(keys.tracks[1].voice.label(Part::Sampler), b"AIRHORN  ");
        keys.release(60);
        keys.press(note(72, 127));
        engine.render(keys.tracks, &mut buses)?;
        let expected = first_frames(include_bytes!("../../../samples/rave/zap.s16le"));
        assert_eq!(buses[Channel::One.source_bus()].map(I24::inner), expected);

        keys.tracks[0].turn_algorithm(View::Scope, None, 1);
        // Open the voiced stab to check its underlying PCM and root.
        keys.tracks[0].sample_program_mut().timbre = Timbre::DRY;
        keys.release(72);
        keys.press(note(55, 127));
        engine.render(keys.tracks, &mut buses)?;
        // At G3 the 14,706 Hz stab runs at twice its native speed: two stored samples
        // for every three DAC frames. Allow one PCM count of Q15 phase rounding.
        let expected = first_frames(include_bytes!("../../../samples/rave/stab.s16le"));
        for (frame, sample) in buses[Channel::One.source_bus()]
            .iter()
            .step_by(3)
            .zip(expected.iter().step_by(2))
        {
            assert!(frame.inner().abs_diff(*sample) <= 256);
        }
        let track = &mut keys.tracks[0];
        for (turn, name) in [
            (1, b"RAVE RISE"),
            (1, b"AMEN BRK "),
            (1, b"AIRHORN  "),
            (-1, b"AMEN BRK "),
        ] {
            track.turn_algorithm(View::Scope, None, turn);
            assert_eq!(track.voice.label(Part::Sampler), name);
        }
        Ok(())
    })
}

#[test]
fn a_one_shot_retriggers_on_a_new_strike_but_not_when_an_older_key_is_uncovered() {
    let mut keys = Keyboard::<4>::new();
    keys.nav.assignments[0] = Assignment::new(Part::Sampler);
    keys.tracks[0] = Track::new(keys.nav.assignments[0]);
    keys.tracks[0].edit_panel(View::SampleEdit, 0, [-1, 0, 0, 0], false);
    keys.press(note(60, 80));
    keys.press(note(64, 90));
    let struck = keys.tracks[0];
    keys.release(64);
    let released = keys.tracks[0];
    assert!(
        !released
            .events(Some(struck))
            .expect("release")
            .iter()
            .any(|e| matches!(e.message, Message::Sampler(SampleMessage::NoteOn(_))))
    );
    keys.press(note(60, 80));
    assert!(
        keys.tracks[0]
            .events(Some(released))
            .expect("restrike")
            .iter()
            .any(|e| e.message == Message::Sampler(SampleMessage::NoteOn(note(60, 80))))
    );
}

#[test]
fn changing_to_one_shot_while_holding_a_loop_still_releases_the_loop() {
    let mut keys = Keyboard::<4>::new();
    keys.nav.assignments[0] = Assignment::new(Part::Sampler);
    keys.tracks[0] = Track::new(keys.nav.assignments[0]);
    keys.tracks[0].edit_panel(View::SampleEdit, 0, [1, 0, 0, 0], false);
    keys.press(note(60, 80));
    keys.press(note(64, 90));
    keys.tracks[0].edit_panel(View::SampleEdit, 0, [-2, 0, 0, 0], false);
    let before = keys.tracks[0];
    keys.release(64);
    assert_eq!(
        keys.tracks[0]
            .events(Some(before))
            .expect("key release")
            .as_slice(),
        &[at(Message::Sampler(SampleMessage::NoteOff(64)))]
    );
}

#[test]
fn edit_sliders_set_loop_trim_and_keyboard_tracking_on_the_audio_voice() {
    let mut track = Track::new(Assignment::new(Part::Sampler));
    let before = track;
    track.edit_panel(View::SampleEdit, 0, [1, 18, -35, 10], false);
    let playback = Playback {
        mode: PlaybackMode::Loop,
        start: 18,
        end: 65,
        release_ms: 0,
    };
    assert_eq!(track.sample_program().playback, playback);
    assert!(
        track
            .events(Some(before))
            .expect("track events")
            .iter()
            .any(|e| e.message == Message::Sampler(SampleMessage::Playback(playback)))
    );
    assert_eq!(track.sample_program().timbre.key_tracking, 10);
    track.edit_panel(View::SampleEdit, 0, [0, i8::MAX, i8::MIN, i8::MAX], false);
    assert_eq!(
        (
            track.sample_program().playback.start,
            track.sample_program().playback.end,
            track.sample_program().timbre.key_tracking
        ),
        (64, 65, 100)
    );
}

#[test]
fn sampler_env_selects_independent_contours_and_recalls_each_sounds_program() {
    let mut nav = Navigation::INITIAL;
    nav.assignments[0] = Assignment::new(Part::Sampler);
    nav.focus = Channel::One;
    let mut track = Track::factory(nav.focus, nav.assignments[0]);
    let horn = track.sample_program();
    track.edit_panel(View::Scope, 0, [-8, 12, 9, -7], false);
    let edited = track.sample_program().timbre;
    assert_eq!(edited.cutoff_hz, 7937);
    assert_eq!(
        (
            edited.resonance,
            edited.env_semitones,
            edited.velocity_semitones
        ),
        (12, 9, 17)
    );
    nav.press(PageButton::Env);
    assert_eq!(nav.view, View::Envelope);
    track.edit_panel(nav.view, 0, [1, 2, -10, 4], false);
    let amp = track.sample_program().timbre.amp;
    assert_ne!(amp, horn.timbre.amp);
    assert_eq!(track.sample_program().timbre.filter, horn.timbre.filter);
    track.edit_panel(nav.view, 0, [2, 3, -6, 1], true);
    assert_eq!(track.envelope_target, EnvelopeTarget::Modulation);
    let program = track.sample_program();
    assert_eq!(program.timbre.amp, amp);
    assert_ne!(program.timbre.filter, horn.timbre.filter);
    track.turn_algorithm(nav.view, None, 1);
    assert_eq!(track.voice.waveform, 0);

    nav.press(PageButton::Env);
    track.turn_algorithm(nav.view, None, -1);
    assert_eq!(track.sample_program().timbre, Timbre::DRY);
    assert_eq!(track.sample_program().playback.mode, PlaybackMode::OneShot);
    nav.press(PageButton::Edit);
    assert_eq!(nav.view, View::SampleEdit);
    track.edit_panel(nav.view, 0, [0, 7, -12, 25], false);
    nav.press(PageButton::Edit);
    assert_eq!(nav.view, View::Scope);
    track.turn_algorithm(nav.view, None, 1);
    assert_eq!(track.sample_program(), program);
}

#[test]
fn replacing_the_808_plays_the_dry_amen_and_follows_live_tempo_without_retriggering()
-> Result<(), Error> {
    // Independent PCM16 reference points from FFmpeg's IMA WAV decoder.
    #[cfg(all(feature = "packed-samples", feature = "packed-cymbals"))]
    let (rate, start, faster) = (
        24400,
        [
            0, -698, -2437, -4982, -5499, -14989, -15394, -8389, -3379, 3197, 9357, 4822,
        ],
        [
            -7660, -619, 9890, 18288, 17543, 15209, 15214, 8122, -692, -14365, -20513, -20397,
        ],
    );
    #[cfg(not(all(feature = "packed-samples", feature = "packed-cymbals")))]
    let (rate, start, faster) = (
        12505,
        [
            0, 699, 606, -9232, -7526, 14021, 2687, -7880, -14931, -7965, -2135, 3704,
        ],
        [
            -9062, -25510, -20158, -14807, 6208, 20495, 24122, 17047, 2600, -15154, -22888, -16519,
        ],
    );

    with_instrument(rate, |engine| {
        let channel = Channel::Three;
        let mut tracks = Track::rack(Navigation::INITIAL.assignments);
        assert_eq!(tracks[channel.index()].assignment.part, Part::Tr808);
        let track = &mut tracks[channel.index()];
        *track = Track::factory(channel, Assignment::new(Part::Sampler));
        track.turn_algorithm(View::Scope, None, -1);
        engine.sequences = [false, false, true, false];
        engine.transport.toggle();
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        for (bpm, reference) in [(122, start), (244, faster)] {
            engine.transport.bpm = bpm;
            engine.render(tracks, &mut buses)?;
            let actual = &buses[channel.source_bus()];
            for (sample, expected) in actual.iter().step_by(7).zip(reference) {
                assert!(
                    sample.inner().abs_diff(expected << 8) <= 256,
                    "{bpm} BPM: {sample:?} vs {expected}"
                );
            }
        }
        engine.transport.toggle();
        engine.render(tracks, &mut buses)?;
        assert!(
            buses[channel.source_bus()]
                .iter()
                .all(|sample| sample.inner() == 0)
        );
        Ok(())
    })
}
