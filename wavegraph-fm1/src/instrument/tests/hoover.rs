use super::*;

#[test]
#[allow(clippy::arithmetic_side_effects)]
fn hoover_stereo_follows_the_strip_fader_trim_and_mono_effects() -> Result<(), Error> {
    with_instrument(44_118, |rack| {
        let mut tracks = Track::rack([Assignment::new(Part::Hoover); CHANNELS]);
        tracks[0].poly_keys[0] = PolyKey {
            note: Some(note(62, 120)),
            strike: 1,
        };
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        let mut mixer = Mixer::DRY;
        rack.set_fx(FX_WITHOUT_DYNAMICS);
        for level in [128, 64, 192] {
            mixer.0[0].level = level;
            rack.set_mixer(mixer);
            for _ in 0..8 {
                rack.render(tracks, &mut buses)?;
            }
            let side = &buses[1];
            assert!(side.iter().any(|s| s.inner().abs() > 128));
            for (f, sample) in side.iter().enumerate() {
                let center = buses[DRY_BUS][f].inner() << DRY_HEADROOM;
                let side =
                    ((sample.inner() * i32::from(level)) >> (7 + DRY_HEADROOM)) << DRY_HEADROOM;
                assert!((buses[MASTER_BUS][f].inner() - (center + side)).abs() <= 2);
                assert!((buses[MASTER_BUS + 1][f].inner() - (center - side)).abs() <= 2);
            }
            assert!(tape_input(&buses, mixer, Channel::One, false).eq(buses[0]));
            assert!(
                tape_input(&buses, mixer, Channel::One, true)
                    .map(I24::inner)
                    .eq(buses[DRY_BUS].map(|x| x.inner() << DRY_HEADROOM))
            );
        }
        mixer.0[0].enabled = false;
        rack.set_mixer(mixer);
        rack.render(tracks, &mut buses)?;
        assert_eq!(buses[MASTER_BUS], [I24::default(); 128]);
        assert_eq!(buses[MASTER_BUS], buses[MASTER_BUS + 1]);
        mixer.0[0].enabled = true;
        rack.set_mixer(mixer);
        // Each serial processor takes the mono strip input, including the Beat FX gate.
        for effect in 0..4 {
            let mut fx = FX_WITHOUT_DYNAMICS;
            tracks[0].inserts = [Insert::None; INSERT_SLOTS];
            match effect {
                0 => tracks[0].inserts[0] = InsertKind::Saturator.initial(Part::Hoover),
                1 => fx.slicers[0].enabled = true,
                2 => fx.filters[0].pick.press_black(1),
                _ => fx.flanger_target = Some(Channel::One),
            }
            rack.set_fx(fx);
            rack.render(tracks, &mut buses)?;
            assert_eq!(buses[MASTER_BUS], buses[MASTER_BUS + 1]);
        }
        rack.set_fx(FX_WITHOUT_DYNAMICS);
        tracks[0].poly_keys = [PolyKey::EMPTY; POLYPHONY];
        rack.set_half_gain(true)?;
        for _ in 0..6 {
            rack.render(tracks, &mut buses)?;
        }
        let mut headphone = [[0; 2]; 128];
        write_master(&buses, true, &mut headphone);
        assert!(
            headphone.iter().any(|[l, r]| l.abs_diff(*r) > 128),
            "stereo key-up tail"
        );
        let mut remaining = &headphone[..];
        capture_master(&buses, true, |frames| {
            let (expected, rest) = remaining.split_at(frames.len());
            assert_eq!(frames, expected);
            remaining = rest;
            frames.len()
        });
        assert!(remaining.is_empty());
        for (i, [l, r]) in headphone.into_iter().enumerate() {
            assert_eq!(
                [l, r],
                [
                    buses[MASTER_BUS][i].inner() >> 1,
                    buses[MASTER_BUS + 1][i].inner() >> 1
                ]
            );
        }
        tracks[0] = Track::factory(Channel::One, Assignment::new(Part::Tr909));
        rack.render(tracks, &mut buses)?;
        assert_eq!(buses[MASTER_BUS], buses[MASTER_BUS + 1]);
        Ok(())
    })
}

#[test]
fn two_hoovers_share_the_master_with_pump_panel_and_the_boot_cue() -> Result<(), Error> {
    with_instrument(12_000, |rack| {
        let mut tracks =
            Track::rack([Part::Hoover, Part::Bass, Part::Mono, Part::Hoover].map(Assignment::new));
        for (i, pitch) in [(0, 62), (3, 67)] {
            tracks[i].poly_keys[0] = PolyKey {
                note: Some(note(pitch, 95)),
                strike: 1,
            };
        }
        let mut preset = tracks[1].bass_preset;
        preset.turn(2 - i8::try_from(preset.index).expect("small preset bank"));
        tracks[1].sync_panel(preset, PRESETS[2].inserts.map(Insert::kind));
        tracks[1].voice.gate.note = Some(note(41, 100));
        tracks[2].voice.gate.note = Some(note(69, 100));
        let mut boot = crate::bumper::Bumper::default();
        boot.start();
        rack.cue = boot.take(128, 512);
        let mut mixer = Mixer::DRY;
        mixer.0[3].level = 64;
        rack.set_mixer(mixer);
        let mut buses = [[I24::default(); 128]; BUS_COUNT];
        // First render also publishes both master effects, stereo faders and acid returns.
        for _ in 0..12 {
            rack.render(tracks, &mut buses)?;
        }
        assert_eq!(rack.acid_levels, [0, 128, 0, 0]);
        assert_eq!(rack.hoover_levels, [128, 0, 0, 64]);
        assert!(buses[3].iter().any(|v| v.inner() != 0), "303 pedal tap");
        assert!(buses[1].iter().any(|v| v.inner() != 0));
        assert!(buses[7].iter().any(|v| v.inner() != 0));
        for (i, sample) in buses[12].iter().enumerate() {
            assert_eq!(
                sample.inner(),
                (buses[1][i].inner() + (buses[7][i].inner() >> 1)) >> DRY_HEADROOM
            );
        }
        assert_ne!(buses[MASTER_BUS], buses[MASTER_BUS + 1]);
        Ok(())
    })
}
