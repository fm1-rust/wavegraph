//! Exercise the safe effects APIs against the mechanical translation from PR #12.

#![cfg(feature = "raw")]

use wavegraph_303::{
    Compressor, DrumFx, DrumFxSettings, Effects, EffectsSettings, LengthMismatch, Reverb,
    StereoFrame, raw,
};

const BLOCK: usize = 47;

fn input() -> [f32; BLOCK] {
    core::array::from_fn(|i| [0.8125, -0.25, 0.0625, -0.75, 0.375][i % 5])
}

#[test]
fn tape_modulation_and_compressor() {
    let mut storage = [StereoFrame::default(); 2048];
    let mut rack = Effects::new(48_000.0, &mut storage).expect("valid tape");
    let mut compressor = Compressor::new(48_000.0);
    let mut raw_storage = [raw::x0x_Fx_Frame::default(); 2048];
    let mut reference = raw::x0x_Fx::default();
    let mut raw_comp = raw::x0x_Compressor::default();
    let bass = input();
    let send = [0.15; BLOCK];
    // SAFETY: Reference delay storage outlives the rack's use and all sample
    // buffers are disjoint and at least BLOCK frames long.
    unsafe {
        reference.Init(48_000.0, raw_storage.as_mut_ptr(), raw_storage.len());
        raw_comp.Init(48_000.0);
        for i in 0..240 {
            let division = if i < 55 { 0 } else { 5 };
            let settings = EffectsSettings {
                drive: 0.71,
                modulation: if i < 90 { 0.22 } else { 0.85 },
                mod_width: 0.8,
                crush_bits: 0.23,
                crush_rate: 0.54,
                delay_free: i >= 170,
                delay_ms: 31.5,
                delay_mix: if i < 200 { 0.7 } else { 0.0 },
                delay_feedback: 0.93,
                bpm: 400.0,
                delay_division: division,
                ..Default::default()
            };
            let raw_settings = raw::x0x_Fx_Settings {
                drive: settings.drive,
                mod_: settings.modulation,
                mod_width: settings.mod_width,
                crush_bits: settings.crush_bits,
                crush_rate: settings.crush_rate,
                dly_free: settings.delay_free,
                dly_free_ms: settings.delay_ms,
                dly_mix: settings.delay_mix,
                dly_fb: settings.delay_feedback,
                bpm: settings.bpm,
                dly_div: if division == 0 { 0 } else { 5 },
                ..Default::default()
            };
            rack.set(&settings);
            reference.Set(&raw_settings);
            let drums = (i < 75).then_some(&send[..]);
            let mut actual = [StereoFrame::default(); BLOCK];
            let (mut left, mut right) = ([0.0; BLOCK], [0.0; BLOCK]);
            if i == 1 {
                assert_eq!(
                    rack.process(&bass, Some(&send[..1]), &mut actual),
                    Err(LengthMismatch)
                );
                assert_eq!(
                    rack.process(&bass, None, &mut actual[..1]),
                    Err(LengthMismatch)
                );
            }
            rack.process(&bass, drums, &mut actual)
                .expect("matching frames");
            reference.Process(
                bass.as_ptr(),
                drums.map_or(core::ptr::null(), <[f32]>::as_ptr),
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                BLOCK,
            );
            assert_stereo(&actual, &left, &right);
            assert_eq!(rack.bad_count(), reference.BadCount());
            let amount = if i < 200 { 0.9 } else { 0.0 };
            compressor.set_amount(amount);
            raw_comp.Set(amount);
            compressor.process(&mut actual);
            raw_comp.Process(left.as_mut_ptr(), right.as_mut_ptr(), BLOCK);
            assert_stereo(&actual, &left, &right);
            assert_eq!(
                compressor.reduction().to_bits(),
                raw_comp.Reduction().to_bits()
            );
        }
    }
}

fn assert_stereo(actual: &[StereoFrame], left: &[f32], right: &[f32]) {
    for ((frame, l), r) in actual.iter().zip(left).zip(right) {
        assert_eq!(
            [frame.left.to_bits(), frame.right.to_bits()],
            [l.to_bits(), r.to_bits()]
        );
    }
}

#[test]
fn drum_envelope_trigger_and_block_phase() {
    let mut drums = DrumFx::new(44_100.0);
    let mut reference = raw::x0x_DrumFx::default();
    // SAFETY: The reference writes exactly the requested prefix of expected.
    unsafe {
        reference.Init(44_100.0);
        for (i, len) in [0, 5, 13, 47, 1, 3, 47, 21, 11].into_iter().enumerate() {
            let s = DrumFxSettings {
                drive: 0.6,
                filter: if i < 4 { 0.2 } else { 0.83 },
                filter_res: 0.77,
                env_amount: 0.9,
                crush_bits: 0.21,
                crush_rate: 0.3,
                ..Default::default()
            };
            reference.Set(&raw::x0x_DrumFx_Settings {
                drive: s.drive,
                filter: s.filter,
                filter_res: s.filter_res,
                env_amount: s.env_amount,
                crush_bits: s.crush_bits,
                crush_rate: s.crush_rate,
                ..Default::default()
            });
            if i % 2 == 0 {
                let offset = if i == 4 { 100 } else { 2 };
                drums.trigger(offset, 1.2);
                reference.Trigger(offset, 1.2);
            }
            let mut actual = input();
            let mut expected = actual;
            drums.process(&s, &mut actual[..len]);
            reference.Process(expected.as_mut_ptr(), len);
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        }
    }
}

#[test]
fn reverb_impulse_and_storage_bounds() {
    let mut storage = vec![0.0; Reverb::FRAMES];
    assert!(Reverb::new(48_000.0, &mut storage[..100]).is_none());
    assert!(Reverb::new(96_000.0, &mut storage).is_none());
    assert!(Reverb::new(f32::MAX, &mut storage).is_none());
    assert!(Effects::new(48_000.0, &mut [StereoFrame::default(); 463]).is_none());
    let mut reverb = Reverb::new(48_000.0, &mut storage).expect("48 kHz fits");
    let mut raw_store = vec![0.0; Reverb::FRAMES];
    let mut reference = raw::x0x_Reverb::default();
    let mut wet = false;
    // SAFETY: The original tank fits raw_store at 48 kHz. Input and output
    // buffers are disjoint and each covers the entire block.
    unsafe {
        reference.Init(48_000.0, raw_store.as_mut_ptr(), raw_store.len());
        for i in 0..750 {
            let mut mono = [0.0; BLOCK];
            if i == 0 {
                mono[0] = 1.0;
            }
            if i == 180 {
                reverb.set_size(0.92);
                reference.Set(0.92);
            }
            let dry = StereoFrame {
                left: 0.125,
                right: -0.25,
            };
            let mut actual = [dry; BLOCK];
            let mut left = [dry.left; BLOCK];
            let mut right = [dry.right; BLOCK];
            if i == 0 {
                assert_eq!(
                    reverb.process_add(&mono, &mut actual[..1]),
                    Err(LengthMismatch)
                );
            }
            reverb
                .process_add(&mono, &mut actual)
                .expect("matching frames");
            reference.Process(mono.as_ptr(), left.as_mut_ptr(), right.as_mut_ptr(), BLOCK);
            for ((frame, l), r) in actual.iter().zip(left).zip(right) {
                assert!((frame.left - l).abs() < 2e-7 && (frame.right - r).abs() < 2e-7);
            }
            wet |= actual.iter().any(|f| *f != dry);
        }
    }
    assert!(wet);
}
