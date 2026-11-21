//! Compare a short accented and sliding phrase with the original C++ renderer.

use wavegraph_303::raw::{x0x_Fx, x0x_Fx_Frame, x0x_Fx_Settings, x0x_Voice, x0x_VoiceParams};

#[test]
fn accent_slide_and_drive() {
    let mut voice = x0x_Voice::default();
    let params = x0x_VoiceParams::default();
    let mut delay = [x0x_Fx_Frame::default(); 1024];
    let mut fx = x0x_Fx::default();
    // SAFETY: The delay buffer stays live and exclusive to this instance.
    unsafe {
        voice.Init(48_000.0);
        fx.Init(48_000.0, delay.as_mut_ptr(), delay.len());
    }

    // Last dry and driven sample of each 48-frame block in the C++ reference.
    let blocks = [
        (Some((36, true, false)), 0.005, -0.038_369_18, -0.038_369_18),
        (Some((43, false, true)), 0.65, -0.019_210_592, 0.005_572_676),
        (None, 1.0, -0.000_839_297_2, 0.098_428_27),
    ];
    for (note, drive, expected_dry, expected_driven) in blocks {
        let settings = x0x_Fx_Settings {
            drive,
            ..Default::default()
        };
        let mut dry = [0.0; 48];
        let mut left = [0.0; 48];
        let mut right = [0.0; 48];
        // SAFETY: All buffers are distinct and cover the requested frames.
        // A null send disables the optional drum input to the delay.
        unsafe {
            match note {
                Some((pitch, accent, slide)) => voice.NoteOn(pitch, accent, slide),
                None => voice.NoteOff(),
            }
            voice.Process(&params, dry.as_mut_ptr(), dry.len());
            fx.Set(&settings);
            fx.Process(
                dry.as_ptr(),
                core::ptr::null(),
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                dry.len(),
            );
            assert_eq!(voice.Gate(), note.is_some());
            assert!(voice.Sounding());
        }
        assert_eq!(left, right);
        if drive == 0.005 {
            assert_eq!(left, dry);
        }
        assert!((dry[47] - expected_dry).abs() < 2e-6);
        assert!((left[47] - expected_driven).abs() < 2e-6);
    }
}
