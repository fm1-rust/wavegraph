// Generated with Cpp2Rust. See README.md for the source and reproduction steps.
pub const kPi_0: f32 = 3.1415927_f32;
pub unsafe fn Clamp_1(mut x: f32, mut lo: f32, mut hi: f32) -> f32 {
    return if ((x) < (lo)) {
        lo
    } else {
        (if ((x) > (hi)) { hi } else { x })
    };
}
pub unsafe fn ClampInt_2(mut x: i32, mut lo: i32, mut hi: i32) -> i32 {
    return if ((x) < (lo)) {
        lo
    } else {
        (if ((x) > (hi)) { hi } else { x })
    };
}
pub unsafe fn FastExp2_3(mut x: f32) -> f32 {
    x = (unsafe { Clamp_1(x, -30_f32, 30_f32) });
    let fl: f32 = (unsafe { libm::floorf(x) });
    let f: f32 = ((x) - (fl));
    let p: f32 = ((1_f32)
        + ((f)
            * ((0.6931472_f32)
                + ((f)
                    * ((0.2402265_f32) + ((f) * ((0.0555041_f32) + ((f) * (0.0096181_f32)))))))));
    return (unsafe { libm::ldexpf(p, (fl as i32)) });
}
pub unsafe fn FastSin2Pi_6(mut ph: f32) -> f32 {
    ph -= (unsafe { libm::floorf(ph) });
    let t: f32 = if ((ph) < (0.5_f32)) {
        ((2_f32) * (ph))
    } else {
        (((2_f32) * (ph)) - (2_f32))
    };
    let y: f32 = (((4_f32) * (t)) * ((1_f32) - (unsafe { libm::fabsf(t) })));
    return (((0.225_f32) * (((y) * (unsafe { libm::fabsf(y) })) - (y))) + (y));
}
pub unsafe fn FastLog2_8(mut x: f32) -> f32 {
    let mut u: anon_9 = anon_9 { f: x };
    let e: f32 = ((((((u.i) >> (23)) & (255_u32)) as i32) - (127)) as f32);
    u.i = (((u.i) & (8388607_u32)) | (1065353216_u32));
    let m: f32 = u.f;
    return (((e) + ((((-0.34484842_f32) * (m)) + (2.0246658_f32)) * (m))) - (1.6748776_f32));
}
pub unsafe fn FastTanh_10(mut x: f32) -> f32 {
    if ((x) > (3_f32)) {
        return 1_f32;
    }
    if ((x) < (-3_f32)) {
        return -1_f32;
    }
    let x2: f32 = ((x) * (x));
    return (((x) * ((27_f32) + (x2))) / ((27_f32) + ((9_f32) * (x2))));
}
pub unsafe fn DriveMakeup_11(mut drive: f32) -> f32 {
    return ((0.135_f32) + ((0.865_f32) * (unsafe { FastExp2_3(((-14.43_f32) * (drive))) })));
}
pub unsafe fn DrumDriveMakeup_12(mut drive: f32) -> f32 {
    return ((0.2_f32) + ((0.8_f32) * (unsafe { FastExp2_3(((-7.21_f32) * (drive))) })));
}
pub unsafe fn SoftLimit_13(mut x: f32) -> f32 {
    let a: f32 = (unsafe { libm::fabsf(x) });
    if ((a) <= (0.75_f32)) {
        return x;
    }
    let over: f32 = (((a) - (0.75_f32)) * (4_f32));
    let y: f32 = ((0.75_f32) + ((0.25_f32) * ((over) / ((1_f32) + (over)))));
    return if ((x) < (0_f32)) { -y } else { y };
}
pub unsafe fn MidiToHz_14(mut note: f32) -> f32 {
    return ((440_f32) * (unsafe { FastExp2_3((((note) - (69_f32)) * ((1_f32) / (12_f32)))) }));
}
pub unsafe fn TauToCoef_15(mut seconds: f32, mut sr: f32) -> f32 {
    if ((seconds) <= (0_f32)) {
        return 1_f32;
    }
    return ((1_f32) - (unsafe { libm::expf(((-1_f32) / ((seconds) * (sr)))) }));
}
pub unsafe fn KnobToExp_17(mut v: f32, mut lo: f32, mut hi: f32) -> f32 {
    return ((lo) * (unsafe { libm::powf(((hi) / (lo)), v) }));
}
#[repr(C)]
#[derive(Copy, Clone)]
pub union anon_9 {
    pub f: f32,
    pub i: u32,
}
impl Default for anon_9 {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}
pub const kFullResonanceGain_19: f32 = 16.5_f32;
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_VoiceParams {
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub env_oct: f32,
    pub decay_s: f32,
    pub accent: f32,
    pub tuning_st: f32,
    pub slide_s: f32,
    pub square: bool,
    pub pulse_width: f32,
    pub max_loop_gain: f32,
    pub stage_drive: f32,
    pub bass_makeup: f32,
    pub ladder_in: f32,
    pub post_hp_hz: f32,
}
impl Default for x0x_VoiceParams {
    fn default() -> Self {
        unsafe {
            x0x_VoiceParams {
                cutoff_hz: 500_f32,
                resonance: 0.5_f32,
                env_oct: 2_f32,
                decay_s: 0.6_f32,
                accent: 0.5_f32,
                tuning_st: 0_f32,
                slide_s: 0.06_f32,
                square: false,
                pulse_width: 0.5_f32,
                max_loop_gain: kFullResonanceGain_19,
                stage_drive: 0.75_f32,
                bass_makeup: 0.5_f32,
                ladder_in: 0.4_f32,
                post_hp_hz: 120_f32,
            }
        }
    }
}
pub const kStagePoles_20: [f32; 4] = [0.128_f32, 1.0382_f32, 2.3254_f32, 3.2356_f32];
pub const kLoopHighPassHz_21: f32 = 12_f32;
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_DiodeLadder {
    rate_: f32,
    s_: [f32; 4],
    G_: [f32; 4],
    y_: f32,
    hp_: f32,
    hp_coef_: f32,
    stage_drive_: f32,
}
impl x0x_DiodeLadder {
    pub unsafe fn Init(&mut self, mut rate: f32) {
        self.rate_ = rate;
        (unsafe { x0x_DiodeLadder::Reset(self) });
        (unsafe { x0x_DiodeLadder::SetLoopHighPass(self, kLoopHighPassHz_21) });
    }
    pub unsafe fn Reset(&mut self) {
        {
            let mut i: i32 = 0;
            while ((i) < (4)) {
                self.s_[(i) as usize] = 0_f32;
                i = i.wrapping_add(1);
            }
        }
        self.y_ = {
            self.hp_ = 0_f32;
            self.hp_
        };
    }
    pub unsafe fn SetCutoff(&mut self, mut wc_hz: f32) {
        wc_hz = (unsafe { Clamp_1(wc_hz, 5_f32, 16000_f32) });
        {
            let mut i: i32 = 0;
            while ((i) < (4)) {
                let f: f32 = (unsafe {
                    Clamp_1(
                        ((wc_hz) * (kStagePoles_20[(i) as usize])),
                        10_f32,
                        ((0.45_f32) * (self.rate_)),
                    )
                });
                let g: f32 = (unsafe { libm::tanf(((kPi_0 * (f)) / (self.rate_))) });
                self.G_[(i) as usize] = ((g) / ((1_f32) + (g)));
                i = i.wrapping_add(1);
            }
        }
    }
    pub unsafe fn SetLoopHighPass(&mut self, mut hz: f32) {
        self.hp_coef_ =
            (unsafe { TauToCoef_15(((1_f32) / (((2_f32) * kPi_0) * (hz))), self.rate_) });
    }
    pub unsafe fn SetStageDrive(&mut self, mut d: f32) {
        self.stage_drive_ = d;
    }
    pub unsafe fn Process(&mut self, mut x: f32, mut k: f32) -> f32 {
        let g01: f32 = ((self.G_[(0) as usize]) * (self.G_[(1) as usize]));
        let g23: f32 = ((self.G_[(2) as usize]) * (self.G_[(3) as usize]));
        let gamma: f32 = ((g01) * (g23));
        let sigma: f32 = (((((((self.G_[(1) as usize]) * (g23))
            * ((1_f32) - (self.G_[(0) as usize])))
            * (self.s_[(0) as usize]))
            + (((g23) * ((1_f32) - (self.G_[(1) as usize]))) * (self.s_[(1) as usize])))
            + (((self.G_[(3) as usize]) * ((1_f32) - (self.G_[(2) as usize])))
                * (self.s_[(2) as usize])))
            + (((1_f32) - (self.G_[(3) as usize])) * (self.s_[(3) as usize])));
        let y_est: f32 =
            ((((gamma) * ((x) + ((k) * (self.hp_)))) + (sigma)) / ((1_f32) + ((k) * (gamma))));
        let in_: f32 = (unsafe { FastTanh_10(((x) - ((k) * ((y_est) - (self.hp_))))) });
        let a: f32 = (unsafe {
            let _x: f32 = (unsafe { x0x_DiodeLadder::Sat(self, in_) });
            let _G: f32 = self.G_[(0) as usize];
            let _z: *mut f32 = &mut self.s_[(0) as usize];
            x0x_DiodeLadder::Stage(_x, _G, _z)
        });
        let b: f32 = (unsafe {
            let _x: f32 = (unsafe { x0x_DiodeLadder::Sat(self, a) });
            let _G: f32 = self.G_[(1) as usize];
            let _z: *mut f32 = &mut self.s_[(1) as usize];
            x0x_DiodeLadder::Stage(_x, _G, _z)
        });
        let c: f32 = (unsafe {
            let _x: f32 = (unsafe { x0x_DiodeLadder::Sat(self, b) });
            let _G: f32 = self.G_[(2) as usize];
            let _z: *mut f32 = &mut self.s_[(2) as usize];
            x0x_DiodeLadder::Stage(_x, _G, _z)
        });
        self.y_ = (unsafe {
            let _x: f32 = (unsafe { x0x_DiodeLadder::Sat(self, c) });
            let _G: f32 = self.G_[(3) as usize];
            let _z: *mut f32 = &mut self.s_[(3) as usize];
            x0x_DiodeLadder::Stage(_x, _G, _z)
        });
        self.hp_ += (((self.y_) - (self.hp_)) * (self.hp_coef_));
        return self.y_;
    }
    unsafe fn Sat(&self, mut x: f32) -> f32 {
        return if ((self.stage_drive_) > (0_f32)) {
            ((unsafe { FastTanh_10(((self.stage_drive_) * (x))) }) / (self.stage_drive_))
        } else {
            x
        };
    }
    unsafe fn Stage(mut x: f32, mut G: f32, z: *mut f32) -> f32 {
        let v: f32 = ((G) * ((x) - (*z)));
        let y: f32 = ((v) + (*z));
        (*z) = ((y) + (v));
        return y;
    }
}
impl Default for x0x_DiodeLadder {
    fn default() -> Self {
        unsafe {
            x0x_DiodeLadder {
                rate_: 96000_f32,
                s_: [0_f32; 4],
                G_: [0.5_f32, 0.5_f32, 0.5_f32, 0.5_f32],
                y_: 0_f32,
                hp_: 0_f32,
                hp_coef_: 0_f32,
                stage_drive_: 0_f32,
            }
        }
    }
}
pub const kMaxLoopGain_23: f32 = kFullResonanceGain_19;
pub const kCutoffScale_24: f32 = 1.6_f32;
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_Voice {
    sr_: f32,
    phase_: f32,
    square_mix_: f32,
    pitch_: f32,
    target_: f32,
    ladder_: x0x_DiodeLadder,
    coef_count_: i32,
    fenv_: f32,
    aenv_: f32,
    acc_cap_: f32,
    acc_amp_: f32,
    post_lp_: f32,
    gate_: bool,
    amp_on_: bool,
    accent_: bool,
    sliding_: bool,
}
impl x0x_Voice {
    pub unsafe fn Init(&mut self, mut sample_rate: f32) {
        self.sr_ = sample_rate;
        (unsafe { x0x_DiodeLadder::Init(&mut self.ladder_, ((2_f32) * (sample_rate))) });
        (unsafe { x0x_Voice::Reset(self) });
    }
    pub unsafe fn Reset(&mut self) {
        self.phase_ = 0_f32;
        (unsafe { x0x_DiodeLadder::Reset(&mut self.ladder_) });
        self.coef_count_ = 0;
        self.fenv_ = {
            self.aenv_ = {
                self.acc_cap_ = 0_f32;
                self.acc_cap_
            };
            self.aenv_
        };
        self.gate_ = {
            self.accent_ = {
                self.sliding_ = false;
                self.sliding_
            };
            self.accent_
        };
    }
    pub unsafe fn NoteOn(&mut self, mut note: i32, mut accent: bool, mut slide: bool) {
        self.target_ = (note as f32);
        self.accent_ = accent;
        if (slide) && (self.gate_) {
            self.sliding_ = true;
        } else {
            self.pitch_ = self.target_;
            self.sliding_ = false;
            self.fenv_ = 1_f32;
            self.amp_on_ = true;
        }
        self.gate_ = true;
    }
    pub unsafe fn NoteOff(&mut self) {
        self.gate_ = false;
    }
    pub unsafe fn Gate(&self) -> bool {
        return self.gate_;
    }
    pub unsafe fn Sounding(&self) -> bool {
        return (self.gate_) || ((self.aenv_) > (1e-04_f32));
    }
    pub unsafe fn Process(&mut self, p: *const x0x_VoiceParams, mut out: *mut f32, mut n: usize) {
        let glide: f32 = if self.sliding_ {
            (unsafe { TauToCoef_15((((*p).slide_s) / (3_f32)), self.sr_) })
        } else {
            1_f32
        };
        let decay: f32 = if self.accent_ { 0.2_f32 } else { (*p).decay_s };
        let fdec: f32 = (unsafe { TauToCoef_15(((decay) / (4.6_f32)), self.sr_) });
        let att: f32 = (unsafe { TauToCoef_15(0.0015_f32, self.sr_) });
        let hold: f32 = (unsafe { TauToCoef_15(1.34_f32, self.sr_) });
        let rel: f32 = (unsafe { TauToCoef_15(0.004_f32, self.sr_) });
        let acc_charge: f32 = (unsafe { TauToCoef_15(0.012_f32, self.sr_) });
        let acc_drain: f32 =
            (unsafe { TauToCoef_15(((0.08_f32) + ((0.35_f32) * ((*p).resonance))), self.sr_) });
        let k: f32 = (((*p).max_loop_gain) * ((*p).resonance));
        (unsafe { x0x_DiodeLadder::SetStageDrive(&mut self.ladder_, (*p).stage_drive) });
        let amp_acc: f32 = if self.accent_ {
            ((1_f32) + ((0.9_f32) * ((*p).accent)))
        } else {
            1_f32
        };
        let acc_glide: f32 = (unsafe { TauToCoef_15(0.002_f32, self.sr_) });
        let wave_fade: f32 = (unsafe { TauToCoef_15(0.0015_f32, self.sr_) });
        let post_hp: f32 = if (((*p).post_hp_hz) > (0_f32)) {
            (unsafe {
                TauToCoef_15(
                    ((1_f32) / (((2_f32) * kPi_0) * ((*p).post_hp_hz))),
                    self.sr_,
                )
            })
        } else {
            0_f32
        };
        let base_oct: f32 = (unsafe { libm::log2f((((*p).cutoff_hz) * kCutoffScale_24)) });
        {
            let mut i: usize = 0_usize;
            while ((i) < (n)) {
                self.pitch_ += (((self.target_) - (self.pitch_)) * (glide));
                let inc: f32 =
                    ((unsafe { MidiToHz_14(((self.pitch_) + ((*p).tuning_st))) }) / (self.sr_));
                self.fenv_ -= ((self.fenv_) * (fdec));
                if self.gate_ {
                    if self.amp_on_ {
                        self.aenv_ += (((1.05_f32) - (self.aenv_)) * (att));
                        if ((self.aenv_) >= (1_f32)) {
                            self.aenv_ = 1_f32;
                            self.amp_on_ = false;
                        }
                    } else {
                        self.aenv_ -= ((self.aenv_) * (hold));
                    }
                } else {
                    self.aenv_ -= ((self.aenv_) * (rel));
                }
                let acc_in: f32 = if self.accent_ { self.fenv_ } else { 0_f32 };
                self.acc_cap_ += (((acc_in) - (self.acc_cap_))
                    * (if ((acc_in) > (self.acc_cap_)) {
                        acc_charge
                    } else {
                        acc_drain
                    }));
                let previous = self.coef_count_;
                self.coef_count_ = previous.wrapping_sub(1);
                if previous <= 0 {
                    self.coef_count_ = 3;
                    let oct: f32 = (((base_oct) + (((*p).env_oct) * (self.fenv_)))
                        + (((3_f32) * ((*p).accent)) * (self.acc_cap_)));
                    (unsafe {
                        x0x_DiodeLadder::SetCutoff(&mut self.ladder_, (unsafe { FastExp2_3(oct) }))
                    });
                }
                self.square_mix_ += (((if (*p).square { 1_f32 } else { 0_f32 })
                    - (self.square_mix_))
                    * (wave_fade));
                let x: f32 = (unsafe {
                    let _square: f32 = self.square_mix_;
                    x0x_Voice::Osc(self, inc, _square, (*p).pulse_width)
                });
                let mut y: f32 = 0_f32;
                {
                    let mut os: i32 = 0;
                    while ((os) < (2)) {
                        y += (unsafe {
                            x0x_DiodeLadder::Process(&mut self.ladder_, ((x) * ((*p).ladder_in)), k)
                        });
                        os = os.wrapping_add(1);
                    }
                }
                y /= (*p).ladder_in;
                y *= ((0.4_f32) * ((1_f32) + (((*p).bass_makeup) * (k))));
                self.acc_amp_ += (((amp_acc) - (self.acc_amp_)) * (acc_glide));
                if ((post_hp) > (0_f32)) {
                    self.post_lp_ += (((y) - (self.post_lp_)) * (post_hp));
                    y = ((1.25_f32) * ((y) - (self.post_lp_)));
                }
                (*out.offset((i) as isize)) +=
                    ((((y) * (self.aenv_)) * (self.acc_amp_)) * (0.265_f32));
                i = i.wrapping_add(1);
            }
        }
    }
    unsafe fn Osc(&mut self, mut inc: f32, mut square: f32, mut pw: f32) -> f32 {
        self.phase_ += inc;
        if ((self.phase_) >= (1_f32)) {
            self.phase_ -= 1_f32;
        }
        let mut saw: f32 = ((((2_f32) * (self.phase_)) - (1_f32))
            - (unsafe { x0x_Voice::Blep(self.phase_, inc) }));
        if ((square) < (1e-04_f32)) {
            return saw;
        }
        let mut p2: f32 = ((self.phase_) + (pw));
        if ((p2) >= (1_f32)) {
            p2 -= 1_f32;
        }
        let saw2: f32 = ((((2_f32) * (p2)) - (1_f32)) - (unsafe { x0x_Voice::Blep(p2, inc) }));
        let pulse: f32 = ((0.7_f32) * ((saw) - (saw2)));
        return if ((square) > (0.9999_f32)) {
            pulse
        } else {
            ((saw) + (((pulse) - (saw)) * (square)))
        };
    }
    unsafe fn Blep(mut t: f32, mut dt: f32) -> f32 {
        if ((t) < (dt)) {
            t /= dt;
            return ((((t) + (t)) - ((t) * (t))) - (1_f32));
        }
        if ((t) > ((1_f32) - (dt))) {
            t = (((t) - (1_f32)) / (dt));
            return (((((t) * (t)) + (t)) + (t)) + (1_f32));
        }
        return 0_f32;
    }
}
impl Default for x0x_Voice {
    fn default() -> Self {
        unsafe {
            x0x_Voice {
                sr_: 48000_f32,
                phase_: 0_f32,
                square_mix_: 0_f32,
                pitch_: 36_f32,
                target_: 36_f32,
                ladder_: <x0x_DiodeLadder>::default(),
                coef_count_: 0,
                fenv_: 0_f32,
                aenv_: 0_f32,
                acc_cap_: 0_f32,
                acc_amp_: 1_f32,
                post_lp_: 0_f32,
                gate_: false,
                amp_on_: false,
                accent_: false,
                sliding_: false,
            }
        }
    }
}
pub const kDelayDivisions_26: i32 = 9;
pub const kDelayBeats_27: [f32; 9] = [
    ((1_f32) / (6_f32)),
    0.25_f32,
    ((1_f32) / (3_f32)),
    0.5_f32,
    ((2_f32) / (3_f32)),
    0.75_f32,
    1_f32,
    1.5_f32,
    2_f32,
];
pub const kLfoShapes_28: i32 = 5;
pub const kLfoRates_29: i32 = 11;
pub const kLfoBeats_30: [f32; 11] = [
    16_f32,
    8_f32,
    4_f32,
    2_f32,
    1_f32,
    ((2_f32) / (3_f32)),
    0.5_f32,
    ((1_f32) / (3_f32)),
    0.25_f32,
    ((1_f32) / (6_f32)),
    0.125_f32,
];
pub type x0x_LfoTarget = u32;
pub const x0x_LfoTarget_LFO_OFF: x0x_LfoTarget = 0;
pub const x0x_LfoTarget_LFO_LEVEL: x0x_LfoTarget = 1;
pub const x0x_LfoTarget_LFO_PAN: x0x_LfoTarget = 2;
pub const x0x_LfoTarget_LFO_TUNE: x0x_LfoTarget = 3;
pub const x0x_LfoTarget_LFO_FILTER: x0x_LfoTarget = 4;
pub const x0x_LfoTarget_LFO_FM: x0x_LfoTarget = 5;
pub const x0x_LfoTarget_LFO_DECAY: x0x_LfoTarget = 6;
pub const x0x_LfoTarget_kLfoTargets: x0x_LfoTarget = 7;
pub unsafe fn LfoValue_31(mut shape: i32, mut phase: f32, mut held: f32) -> f32 {
    {
        match { shape } {
            __v if __v == 1 => {
                return ((1_f32) - ((4_f32) * (unsafe { libm::fabsf(((phase) - (0.5_f32))) })));
            }
            __v if __v == 2 => {
                return (((2_f32) * (phase)) - (1_f32));
            }
            __v if __v == 3 => {
                return ((1_f32) - ((2_f32) * (phase)));
            }
            __v if __v == 4 => {
                return held;
            }
            _ => {
                return 0_f32;
            }
        }
    };
    panic!("ub: non-void function does not return a value")
}
pub const kDelayFreeMinMs_32: f32 = 30_f32;
pub const kDelayFreeMaxMs_33: f32 = 1900_f32;
const kModSize_34: i32 = 2048;
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct x0x_Fx_Frame {
    pub l: f32,
    pub r: f32,
}
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_Fx_Settings {
    pub drive: f32,
    pub crush_bits: f32,
    pub crush_rate: f32,
    pub mod_: f32,
    pub mod_width: f32,
    pub dly_mix: f32,
    pub dly_send: f32,
    pub dly_div: i32,
    pub dly_free: bool,
    pub dly_free_ms: f32,
    pub dly_fb: f32,
    pub dly_tone: f32,
    pub bpm: f32,
}
impl Default for x0x_Fx_Settings {
    fn default() -> Self {
        unsafe {
            x0x_Fx_Settings {
                drive: 0_f32,
                crush_bits: 0_f32,
                crush_rate: 0_f32,
                mod_: 0_f32,
                mod_width: 0.5_f32,
                dly_mix: 0_f32,
                dly_send: 1_f32,
                dly_div: 5,
                dly_free: false,
                dly_free_ms: 375_f32,
                dly_fb: 0.35_f32,
                dly_tone: 0.4_f32,
                bpm: 120_f32,
            }
        }
    }
}
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_Fx {
    sr_: f32,
    s_: x0x_Fx_Settings,
    hp_state_: f32,
    pre_state_: f32,
    dc_: f32,
    tone1_: f32,
    tone2_: f32,
    held_: f32,
    hold_count_: i32,
    lfo_: f32,
    mod_buf_: [f32; 2048],
    mod_pos_: i32,
    delay_: *mut x0x_Fx_Frame,
    delay_size_: usize,
    delay_pos_: usize,
    target_delay_: f32,
    head_a_: f32,
    head_b_: f32,
    fade_: f32,
    fading_: bool,
    heads_set_: bool,
    delay_tail_: i32,
    wow_: f32,
    flutter_: f32,
    lp_l_: f32,
    lp_r_: f32,
    hpl_: f32,
    hpr_: f32,
    bad_: u32,
}
impl x0x_Fx {
    pub unsafe fn Init(
        &mut self,
        mut sample_rate: f32,
        mut delay_mem: *mut x0x_Fx_Frame,
        mut delay_frames: usize,
    ) {
        self.sr_ = sample_rate;
        self.delay_ = delay_mem;
        self.delay_size_ = delay_frames;
        {
            let mut i: usize = 0_usize;
            while ((i) < (self.delay_size_)) {
                (*self.delay_.offset((i) as isize)) = x0x_Fx_Frame { l: 0_f32, r: 0_f32 };
                i = i.wrapping_add(1);
            }
        }
        {
            let mut i: i32 = 0;
            while ((i) < kModSize_34) {
                self.mod_buf_[(i) as usize] = 0_f32;
                i = i.wrapping_add(1);
            }
        }
        self.delay_pos_ = ({
            self.mod_pos_ = 0;
            self.mod_pos_
        } as usize);
        self.heads_set_ = false;
    }
    pub unsafe fn Set(&mut self, s: *const x0x_Fx_Settings) {
        self.s_ = (*s);
        let mut samples: f32 = 0_f32;
        if (*s).dly_free {
            samples =
                (((unsafe { Clamp_1((*s).dly_free_ms, kDelayFreeMinMs_32, kDelayFreeMaxMs_33) })
                    * (0.001_f32))
                    * (self.sr_));
        } else {
            samples = ((((kDelayBeats_27
                [(unsafe { ClampInt_2((*s).dly_div, 0, (kDelayDivisions_26 - (1))) }) as usize])
                * (60_f32))
                / ((*s).bpm))
                * (self.sr_));
        }
        self.target_delay_ = (unsafe {
            Clamp_1(
                samples,
                64_f32,
                (((self.delay_size_).wrapping_sub(400_usize)) as f32),
            )
        });
        if !(self.heads_set_) {
            {
                self.head_a_ = {
                    self.head_b_ = self.target_delay_;
                    self.head_b_
                };
                self.heads_set_ = true
            };
        }
    }
    pub unsafe fn Process(
        &mut self,
        mut in_: *const f32,
        mut send: *const f32,
        mut left: *mut f32,
        mut right: *mut f32,
        mut n: usize,
    ) {
        let driving: bool = ((self.s_.drive) > (0.005_f32));
        let gain: f32 = ((1_f32) + (((60_f32) * (self.s_.drive)) * (self.s_.drive)));
        let bias: f32 = ((0.08_f32) * (self.s_.drive));
        let makeup: f32 = (unsafe { DriveMakeup_11(self.s_.drive) });
        let hp_in: f32 =
            (unsafe { TauToCoef_15(((1_f32) / (((2_f32) * kPi_0) * (120_f32))), self.sr_) });
        let pre_lp: f32 =
            (unsafe { TauToCoef_15(((1_f32) / (((2_f32) * kPi_0) * (1000_f32))), self.sr_) });
        let tone_lp: f32 = (unsafe {
            let _seconds: f32 =
                ((1_f32) / (((2_f32) * kPi_0) * ((8000_f32) - ((2000_f32) * (self.s_.drive)))));
            let _sr: f32 = self.sr_;
            TauToCoef_15(_seconds, _sr)
        });
        let crush_bits: bool = ((self.s_.crush_bits) > (0.005_f32));
        let levels: f32 = (unsafe { FastExp2_3(((15_f32) - ((12_f32) * (self.s_.crush_bits)))) });
        let hold: i32 = ((1) + ((((31_f32) * (self.s_.crush_rate)) * (self.s_.crush_rate)) as i32));
        let modding: bool = ((self.s_.mod_) > (0.005_f32));
        let w: f32 = self.s_.mod_width;
        let mut base_ms: f32 = 0_f32;
        let mut depth_ms: f32 = 0_f32;
        let mut rate_hz: f32 = 0_f32;
        let mut fb: f32 = 0_f32;
        let mut wet: f32 = 0_f32;
        if ((self.s_.mod_) < (0.5_f32)) {
            let t: f32 = ((self.s_.mod_) * (2_f32));
            {
                {
                    {
                        {
                            base_ms = 8_f32;
                            depth_ms = ((2_f32) + ((3_f32) * (w)))
                        };
                        rate_hz = 0.6_f32
                    };
                    fb = 0_f32
                };
                wet = ((0.4_f32) + ((0.3_f32) * (t)))
            };
        } else {
            let t: f32 = (((self.s_.mod_) - (0.5_f32)) * (2_f32));
            {
                {
                    {
                        {
                            base_ms = 1.5_f32;
                            depth_ms = ((1_f32) + ((1_f32) * (w)))
                        };
                        rate_hz = 0.18_f32
                    };
                    fb = ((0.3_f32) + ((0.5_f32) * (t)))
                };
                wet = 0.7_f32
            };
        }
        let base: f32 = (((base_ms) * (0.001_f32)) * (self.sr_));
        let depth: f32 = (((depth_ms) * (0.001_f32)) * (self.sr_));
        let lfo_inc: f32 = ((rate_hz) / (self.sr_));
        let phase_r: f32 = ((0.5_f32) * (w));
        let sending: bool = ((self.s_.dly_mix) > (0.005_f32)) || (!((send).is_null()));
        let delaying: bool = (sending) || ((self.delay_tail_) > (0));
        let mut dry: f32 = 0_f32;
        let mut dwet: f32 = 0_f32;
        (unsafe {
            x0x_Fx::MixGains(
                self.s_.dly_mix,
                (&mut dry as *mut f32),
                (&mut dwet as *mut f32),
            )
        });
        let dfb: f32 = ((1.05_f32) * (self.s_.dly_fb));
        let tape_lp: f32 = (unsafe {
            let _seconds: f32 = ((1_f32)
                / (((2_f32) * kPi_0)
                    * ((900_f32) + (((6000_f32) * (self.s_.dly_tone)) * (self.s_.dly_tone)))));
            let _sr: f32 = self.sr_;
            TauToCoef_15(_seconds, _sr)
        });
        let tape_hp: f32 =
            (unsafe { TauToCoef_15(((1_f32) / (((2_f32) * kPi_0) * (90_f32))), self.sr_) });
        let glide: f32 = (unsafe {
            let _seconds: f32 = if self.s_.dly_free { 0.15_f32 } else { 0.05_f32 };
            let _sr: f32 = self.sr_;
            TauToCoef_15(_seconds, _sr)
        });
        let fade_in: f32 = ((1_f32) / ((0.08_f32) * (self.sr_)));
        {
            let mut i: usize = 0_usize;
            while ((i) < (n)) {
                let mut x: f32 = (*in_.offset((i) as isize));
                if driving {
                    self.hp_state_ += (((x) - (self.hp_state_)) * (hp_in));
                    let tight: f32 = ((x) - (self.hp_state_));
                    self.pre_state_ += (((tight) - (self.pre_state_)) * (pre_lp));
                    let bright: f32 = ((tight) + ((tight) - (self.pre_state_)));
                    let mut d: f32 = (((bright) * (gain)) + (bias));
                    d = (unsafe { x0x_Fx::HardClip(d) });
                    self.dc_ += (((d) - (self.dc_)) * (5e-04_f32));
                    d -= self.dc_;
                    self.tone1_ += (((d) - (self.tone1_)) * (tone_lp));
                    self.tone2_ += (((self.tone1_) - (self.tone2_)) * (tone_lp));
                    x = ((self.tone2_) * (makeup));
                }
                if (crush_bits) || ((hold) > (1)) {
                    let previous = self.hold_count_;
                    self.hold_count_ = previous.wrapping_sub(1);
                    if previous <= 0 {
                        self.hold_count_ = ((hold) - (1));
                        self.held_ = if crush_bits {
                            ((unsafe {
                                libm::floorf(
                                    (((unsafe { Clamp_1(x, -1_f32, 1_f32) }) * (levels))
                                        + (0.5_f32)),
                                )
                            }) / (levels))
                        } else {
                            x
                        };
                    }
                    x = self.held_;
                }
                let mut l: f32 = x;
                let mut r: f32 = x;
                if modding {
                    self.lfo_ += lfo_inc;
                    if ((self.lfo_) >= (1_f32)) {
                        self.lfo_ -= 1_f32;
                    }
                    let mut pr: f32 = ((self.lfo_) + (phase_r));
                    if ((pr) >= (1_f32)) {
                        pr -= 1_f32;
                    }
                    let tl: f32 = (unsafe {
                        let _delay: f32 = ((base)
                            + ((depth)
                                * ((0.5_f32)
                                    + ((0.5_f32) * (unsafe { x0x_Fx::Sine(self.lfo_) })))));
                        x0x_Fx::ModTap(self, _delay)
                    });
                    let tr: f32 = (unsafe {
                        x0x_Fx::ModTap(
                            self,
                            ((base)
                                + ((depth)
                                    * ((0.5_f32) + ((0.5_f32) * (unsafe { x0x_Fx::Sine(pr) }))))),
                        )
                    });
                    self.mod_buf_[(self.mod_pos_) as usize] =
                        ((x) + (((fb) * (0.5_f32)) * ((tl) + (tr))));
                    self.mod_pos_ = (((self.mod_pos_) + (1)) % kModSize_34);
                    let norm: f32 = ((1_f32) / ((1_f32) + ((0.5_f32) * (wet))));
                    l = (((x) + ((wet) * (tl))) * (norm));
                    r = (((x) + ((wet) * (tr))) * (norm));
                }
                if delaying {
                    let far: bool =
                        ((unsafe { libm::fabsf(((self.target_delay_) - (self.head_a_))) })
                            > ((0.03_f32) * (self.head_a_)));
                    if self.fading_ {
                        self.fade_ += fade_in;
                        if ((self.fade_) >= (1_f32)) {
                            {
                                {
                                    self.head_a_ = self.head_b_;
                                    self.fade_ = 0_f32
                                };
                                self.fading_ = false
                            };
                        }
                    } else if (far) && (!(self.s_.dly_free)) {
                        {
                            {
                                self.head_b_ = self.target_delay_;
                                self.fade_ = 0_f32
                            };
                            self.fading_ = true
                        };
                    } else {
                        let step: f32 = (unsafe {
                            Clamp_1(
                                (((self.target_delay_) - (self.head_a_)) * (glide)),
                                -0.33_f32,
                                0.33_f32,
                            )
                        });
                        self.head_a_ += step;
                    }
                    self.wow_ += ((0.55_f32) / (self.sr_));
                    if ((self.wow_) >= (1_f32)) {
                        self.wow_ -= 1_f32;
                    }
                    self.flutter_ += ((6.5_f32) / (self.sr_));
                    if ((self.flutter_) >= (1_f32)) {
                        self.flutter_ -= 1_f32;
                    }
                    let wobble: f32 = ((self.sr_)
                        * (((0.0012_f32) * (unsafe { x0x_Fx::Sine(self.wow_) }))
                            + ((0.00015_f32) * (unsafe { x0x_Fx::Sine(self.flutter_) }))));
                    let mut d: x0x_Fx_Frame = (unsafe {
                        let _delay: f32 = ((self.head_a_) + (wobble));
                        x0x_Fx::DelayTap(self, _delay)
                    });
                    if !(((unsafe { libm::fabsf(d.l) }) + (unsafe { libm::fabsf(d.r) }))
                        < (100_f32))
                    {
                        {
                            d = x0x_Fx_Frame { l: 0_f32, r: 0_f32 };
                            self.bad_ = self.bad_.wrapping_add(1)
                        };
                    }
                    if self.fading_ {
                        let b: x0x_Fx_Frame = (unsafe {
                            let _delay: f32 = ((self.head_b_) + (wobble));
                            x0x_Fx::DelayTap(self, _delay)
                        });
                        let gb: f32 = (unsafe { FastSin2Pi_6(((0.25_f32) * (self.fade_))) });
                        let ga: f32 =
                            (unsafe { FastSin2Pi_6(((0.25_f32) - ((0.25_f32) * (self.fade_)))) });
                        d = x0x_Fx_Frame {
                            l: (((d.l) * (ga)) + ((b.l) * (gb))),
                            r: (((d.r) * (ga)) + ((b.r) * (gb))),
                        };
                    }
                    let in_d: f32 = (((((self.s_.dly_send) * (dwet)) * ((l) + (r))) * (0.5_f32))
                        + (if !(send).is_null() {
                            (*send.offset((i) as isize))
                        } else {
                            0_f32
                        }));
                    let mut fl: f32 = ((in_d) + ((dfb) * (d.r)));
                    let mut fr: f32 = ((dfb) * (d.l));
                    self.lp_l_ += (((fl) - (self.lp_l_)) * (tape_lp));
                    self.lp_r_ += (((fr) - (self.lp_r_)) * (tape_lp));
                    self.hpl_ += (((self.lp_l_) - (self.hpl_)) * (tape_hp));
                    self.hpr_ += (((self.lp_r_) - (self.hpr_)) * (tape_hp));
                    (*self.delay_.offset((self.delay_pos_) as isize)) = x0x_Fx_Frame {
                        l: (unsafe { FastTanh_10(((self.lp_l_) - (self.hpl_))) }),
                        r: (unsafe { FastTanh_10(((self.lp_r_) - (self.hpr_))) }),
                    };
                    self.delay_pos_ =
                        ((self.delay_pos_).wrapping_add(1_usize)).wrapping_rem(self.delay_size_);
                    l = (((l) * (dry)) + (d.l));
                    r = (((r) * (dry)) + (d.r));
                    if sending {
                        self.delay_tail_ = (((self.sr_) * (6_f32)) as i32);
                    } else if ((self.delay_tail_) > (0)) {
                        self.delay_tail_ = self.delay_tail_.wrapping_sub(1);
                    }
                }
                (*left.offset((i) as isize)) = l;
                (*right.offset((i) as isize)) = r;
                i = i.wrapping_add(1);
            }
        }
        if !((((((((((((unsafe { libm::fabsf(self.lp_l_) })
            + (unsafe { libm::fabsf(self.lp_r_) }))
            + (unsafe { libm::fabsf(self.hpl_) }))
            + (unsafe { libm::fabsf(self.hpr_) }))
            + (unsafe { libm::fabsf(self.hp_state_) }))
            + (unsafe { libm::fabsf(self.pre_state_) }))
            + (unsafe { libm::fabsf(self.tone1_) }))
            + (unsafe { libm::fabsf(self.tone2_) }))
            + (unsafe { libm::fabsf(self.dc_) }))
            + (unsafe { libm::fabsf(self.held_) }))
            + (unsafe { libm::fabsf(self.mod_buf_[(self.mod_pos_) as usize]) }))
            < (1e+06_f32))
        {
            self.lp_l_ = {
                self.lp_r_ = {
                    self.hpl_ = {
                        self.hpr_ = {
                            self.hp_state_ = {
                                self.pre_state_ = {
                                    self.tone1_ = {
                                        self.tone2_ = {
                                            self.dc_ = {
                                                self.held_ = 0_f32;
                                                self.held_
                                            };
                                            self.dc_
                                        };
                                        self.tone2_
                                    };
                                    self.tone1_
                                };
                                self.pre_state_
                            };
                            self.hp_state_
                        };
                        self.hpr_
                    };
                    self.hpl_
                };
                self.lp_r_
            };
            {
                let mut i: i32 = 0;
                while ((i) < kModSize_34) {
                    self.mod_buf_[(i) as usize] = 0_f32;
                    i = i.wrapping_add(1);
                }
            }
            self.bad_ = self.bad_.wrapping_add(1);
        }
    }
    pub unsafe fn BadCount(&self) -> u32 {
        return self.bad_;
    }
    unsafe fn Sine(mut ph: f32) -> f32 {
        return (unsafe { FastSin2Pi_6(ph) });
    }
    unsafe fn HardClip(mut x: f32) -> f32 {
        let a: f32 = (unsafe { libm::fabsf(x) });
        if ((a) <= (0.9_f32)) {
            return x;
        }
        let y: f32 =
            ((0.9_f32) + ((0.1_f32) * (unsafe { FastTanh_10((((a) - (0.9_f32)) * (10_f32))) })));
        return if ((x) < (0_f32)) { -y } else { y };
    }
    unsafe fn MixGains(mut m: f32, mut dry: *mut f32, mut wet: *mut f32) {
        m = (unsafe { Clamp_1(m, 0_f32, 1_f32) });
        (*wet) = if ((m) < (0.5_f32)) {
            ((2_f32) * (m))
        } else {
            1_f32
        };
        (*dry) = if ((m) > (0.5_f32)) {
            ((2_f32) * ((1_f32) - (m)))
        } else {
            1_f32
        };
    }
    unsafe fn ModTap(&self, mut delay: f32) -> f32 {
        let mut pos: f32 = ((self.mod_pos_ as f32) - (delay));
        while ((pos) < (0_f32)) {
            pos += (kModSize_34 as f32);
        }
        let i0: i32 = (pos as i32);
        let f: f32 = ((pos) - (i0 as f32));
        let a: f32 = self.mod_buf_[((i0) % kModSize_34) as usize];
        let b: f32 = self.mod_buf_[(((i0) + (1)) % kModSize_34) as usize];
        return ((a) + (((b) - (a)) * (f)));
    }
    unsafe fn DelayTap(&self, mut delay: f32) -> x0x_Fx_Frame {
        let mut pos: f32 = ((self.delay_pos_ as f32) - (delay));
        while ((pos) < (0_f32)) {
            pos += (self.delay_size_ as f32);
        }
        let i0: usize = (pos as usize);
        let f: f32 = ((pos) - (i0 as f32));
        let a: *const x0x_Fx_Frame = &(*self
            .delay_
            .offset(((i0).wrapping_rem(self.delay_size_)) as isize));
        let b: *const x0x_Fx_Frame = &(*self
            .delay_
            .offset((((i0).wrapping_add(1_usize)).wrapping_rem(self.delay_size_)) as isize));
        return x0x_Fx_Frame {
            l: (((*a).l) + ((((*b).l) - ((*a).l)) * (f))),
            r: (((*a).r) + ((((*b).r) - ((*a).r)) * (f))),
        };
    }
}
impl Default for x0x_Fx {
    fn default() -> Self {
        unsafe {
            x0x_Fx {
                sr_: 48000_f32,
                s_: <x0x_Fx_Settings>::default(),
                hp_state_: 0_f32,
                pre_state_: 0_f32,
                dc_: 0_f32,
                tone1_: 0_f32,
                tone2_: 0_f32,
                held_: 0_f32,
                hold_count_: 0,
                lfo_: 0_f32,
                mod_buf_: [0_f32; 2048],
                mod_pos_: 0,
                delay_: core::ptr::null_mut(),
                delay_size_: 1_usize,
                delay_pos_: 0_usize,
                target_delay_: 12000_f32,
                head_a_: 12000_f32,
                head_b_: 12000_f32,
                fade_: 0_f32,
                fading_: false,
                heads_set_: false,
                delay_tail_: 0,
                wow_: 0_f32,
                flutter_: 0_f32,
                lp_l_: 0_f32,
                lp_r_: 0_f32,
                hpl_: 0_f32,
                hpr_: 0_f32,
                bad_: 0_u32,
            }
        }
    }
}
pub const kReverbFrames_35: usize = 65536_usize;
const kInAp_36: [f32; 4] = [142_f32, 107_f32, 379_f32, 277_f32];
const kModAp_37: [f32; 2] = [672_f32, 908_f32];
const kDelay1_38: [f32; 2] = [4453_f32, 4217_f32];
const kTankAp_39: [f32; 2] = [1800_f32, 2656_f32];
const kDelay2_40: [f32; 2] = [3720_f32, 3163_f32];
#[repr(C)]
#[derive(Copy, Clone)]
struct x0x_Reverb_Line {
    pub buf: *mut f32,
    pub size: i32,
    pub len: i32,
    pub pos: i32,
}
impl x0x_Reverb_Line {
    pub unsafe fn Read(&self, mut delay: i32) -> f32 {
        let mut p: i32 = ((self.pos) - (delay));
        while ((p) < (0)) {
            p += self.size;
        }
        return (*self.buf.offset((p) as isize));
    }
    pub unsafe fn Tap(&self, mut delay: f32) -> f32 {
        return (unsafe { x0x_Reverb_Line::Read(self, (delay as i32)) });
    }
    pub unsafe fn ReadFrac(&self, mut delay: f32) -> f32 {
        let di: i32 = (delay as i32);
        let f: f32 = ((delay) - (di as f32));
        let a: f32 = (unsafe { x0x_Reverb_Line::Read(self, di) });
        let b: f32 = (unsafe { x0x_Reverb_Line::Read(self, ((di) + (1))) });
        return ((a) + (((b) - (a)) * (f)));
    }
    pub unsafe fn Write(&mut self, mut v: f32) {
        (*self.buf.offset((self.pos) as isize)) = v;
        self.pos = self.pos.wrapping_add(1);
        if self.pos >= self.size {
            self.pos = 0;
        }
    }
    pub unsafe fn Allpass(&mut self, mut x: f32, mut g: f32) -> f32 {
        let d: f32 = (unsafe {
            let _delay: i32 = self.len;
            x0x_Reverb_Line::Read(self, _delay)
        });
        let v: f32 = ((x) + ((g) * (d)));
        (unsafe { x0x_Reverb_Line::Write(self, v) });
        return ((d) - ((g) * (v)));
    }
}
impl Default for x0x_Reverb_Line {
    fn default() -> Self {
        unsafe {
            x0x_Reverb_Line {
                buf: core::ptr::null_mut(),
                size: 1,
                len: 1,
                pos: 0,
            }
        }
    }
}
#[repr(C)]
#[derive(Copy, Clone)]
struct x0x_Reverb_Half {
    pub mod_ap: x0x_Reverb_Line,
    pub d1: x0x_Reverb_Line,
    pub ap: x0x_Reverb_Line,
    pub d2: x0x_Reverb_Line,
    pub lp: f32,
}
impl Default for x0x_Reverb_Half {
    fn default() -> Self {
        unsafe {
            x0x_Reverb_Half {
                mod_ap: <x0x_Reverb_Line>::default(),
                d1: <x0x_Reverb_Line>::default(),
                ap: <x0x_Reverb_Line>::default(),
                d2: <x0x_Reverb_Line>::default(),
                lp: 0_f32,
            }
        }
    }
}
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_Reverb {
    sr_: f32,
    k_: f32,
    tap_k_: f32,
    mem_: *mut f32,
    ok_: bool,
    pre_: x0x_Reverb_Line,
    in_ap_: [x0x_Reverb_Line; 4],
    tank_: [x0x_Reverb_Half; 2],
    size_: f32,
    decay_: f32,
    damp_: f32,
    bw_: f32,
    bw2_: f32,
    bw_coef_: f32,
    lfo_: [f32; 2],
}
impl x0x_Reverb {
    pub unsafe fn Init(&mut self, mut sample_rate: f32, mut mem: *mut f32, mut frames: usize) {
        self.sr_ = sample_rate;
        self.k_ = ((self.sr_) / (29761_f32));
        self.mem_ = if ((frames) >= kReverbFrames_35) {
            mem
        } else {
            core::ptr::null_mut()
        };
        if !(!(self.mem_).is_null()) {
            return;
        }
        {
            let mut i: usize = 0_usize;
            while ((i) < kReverbFrames_35) {
                (*self.mem_.offset((i) as isize)) = 0_f32;
                i = i.wrapping_add(1);
            }
        }
        let mut at: usize = 0_usize;
        let scale = self.k_;
        let memory = self.mem_;
        let mut take = |l: *mut x0x_Reverb_Line, base: f32| {
            (*l).size = (((((base) * (scale)) * (1.25_f32)) as i32) + (64));
            (*l).buf = memory.offset(at as isize);
            at = at.wrapping_add(((*l).size as usize));
        };
        (unsafe {
            take(
                &mut self.pre_,
                ((((0.04_f32) * (29761_f32)) / (1.25_f32)) + (1_f32)),
            )
        });
        {
            let mut d: i32 = 0;
            while ((d) < (4)) {
                (unsafe {
                    let _l: *mut x0x_Reverb_Line = &mut self.in_ap_[(d) as usize];
                    let _base: f32 = kInAp_36[(d) as usize];
                    take(_l, _base)
                });
                d = d.wrapping_add(1);
            }
        }
        {
            let mut h: i32 = 0;
            while ((h) < (2)) {
                (unsafe {
                    let _l: *mut x0x_Reverb_Line = &mut self.tank_[(h) as usize].mod_ap;
                    let _base: f32 = kModAp_37[(h) as usize];
                    take(_l, _base)
                });
                (unsafe {
                    let _l: *mut x0x_Reverb_Line = &mut self.tank_[(h) as usize].d1;
                    let _base: f32 = kDelay1_38[(h) as usize];
                    take(_l, _base)
                });
                (unsafe {
                    let _l: *mut x0x_Reverb_Line = &mut self.tank_[(h) as usize].ap;
                    let _base: f32 = kTankAp_39[(h) as usize];
                    take(_l, _base)
                });
                (unsafe {
                    let _l: *mut x0x_Reverb_Line = &mut self.tank_[(h) as usize].d2;
                    let _base: f32 = kDelay2_40[(h) as usize];
                    take(_l, _base)
                });
                h = h.wrapping_add(1);
            }
        }
        self.ok_ = ((at) <= kReverbFrames_35);
        (unsafe { x0x_Reverb::Set(self, 0.5_f32) });
    }
    pub unsafe fn Set(&mut self, mut size: f32) {
        size = (unsafe { Clamp_1(size, 0_f32, 1_f32) });
        if ((unsafe { libm::fabsf(((size) - (self.size_))) }) < (1e-04_f32)) {
            return;
        }
        self.size_ = size;
        let sc: f32 = ((0.75_f32) + ((0.5_f32) * (size)));
        let rt60: f32 = ((0.4_f32) * (unsafe { FastExp2_3(((3.9_f32) * (size))) }));
        {
            let mut d: i32 = 0;
            while ((d) < (4)) {
                self.in_ap_[(d) as usize].len = (((kInAp_36[(d) as usize]) * (self.k_)) as i32);
                d = d.wrapping_add(1);
            }
        }
        let mut loop_: f32 = 0_f32;
        {
            let mut h: i32 = 0;
            while ((h) < (2)) {
                let t: *mut x0x_Reverb_Half = &mut self.tank_[(h) as usize];
                (*t).mod_ap.len = ((((kModAp_37[(h) as usize]) * (self.k_)) * (sc)) as i32);
                (*t).d1.len = ((((kDelay1_38[(h) as usize]) * (self.k_)) * (sc)) as i32);
                (*t).ap.len = ((((kTankAp_39[(h) as usize]) * (self.k_)) * (sc)) as i32);
                (*t).d2.len = ((((kDelay2_40[(h) as usize]) * (self.k_)) * (sc)) as i32);
                loop_ += ((((((*t).mod_ap.len) + ((*t).d1.len)) + ((*t).ap.len)) + ((*t).d2.len))
                    as f32);
                h = h.wrapping_add(1);
            }
        }
        self.decay_ = (unsafe {
            libm::fminf(
                (unsafe {
                    libm::powf(
                        10_f32,
                        (((-3_f32) * (loop_)) / (((4_f32) * (rt60)) * (self.sr_))),
                    )
                }),
                0.97_f32,
            )
        });
        self.pre_.len = ((((0.01_f32) + ((0.03_f32) * (size))) * (self.sr_)) as i32);
        self.damp_ = (unsafe {
            TauToCoef_15(
                ((1_f32) / (((2_f32) * kPi_0) * ((6000_f32) - ((3500_f32) * (size))))),
                self.sr_,
            )
        });
        self.bw_coef_ = (unsafe {
            TauToCoef_15(
                ((1_f32) / (((2_f32) * kPi_0) * ((8000_f32) - ((3000_f32) * (size))))),
                self.sr_,
            )
        });
        self.tap_k_ = ((self.k_) * (sc));
    }
    pub unsafe fn Process(
        &mut self,
        mut in_: *const f32,
        mut left: *mut f32,
        mut right: *mut f32,
        mut n: usize,
    ) {
        if (!(!(self.mem_).is_null())) || (!(self.ok_)) {
            return;
        }
        let lfo_inc0: f32 = ((0.6_f32) / (self.sr_));
        let lfo_inc1: f32 = ((0.83_f32) / (self.sr_));
        let swing: f32 = ((16_f32) * (self.k_));
        {
            let mut i: usize = 0_usize;
            while ((i) < (n)) {
                let mut x: f32 = (unsafe {
                    let _delay: i32 = self.pre_.len;
                    x0x_Reverb_Line::Read(&self.pre_, _delay)
                });
                (unsafe {
                    let _v: f32 = (*in_.offset((i) as isize));
                    x0x_Reverb_Line::Write(&mut self.pre_, _v)
                });
                self.bw_ += (((x) - (self.bw_)) * (self.bw_coef_));
                self.bw2_ += (((self.bw_) - (self.bw2_)) * (self.bw_coef_));
                x = self.bw2_;
                {
                    let mut d: i32 = 0;
                    while ((d) < (4)) {
                        x = (unsafe {
                            let _g: f32 = if ((d) < (2)) { 0.75_f32 } else { 0.625_f32 };
                            x0x_Reverb_Line::Allpass(&mut self.in_ap_[(d) as usize], x, _g)
                        });
                        d = d.wrapping_add(1);
                    }
                }
                {
                    self.lfo_[(0) as usize] += lfo_inc0;
                    self.lfo_[(1) as usize] += lfo_inc1
                };
                {
                    let mut h: i32 = 0;
                    while ((h) < (2)) {
                        if ((self.lfo_[(h) as usize]) >= (1_f32)) {
                            self.lfo_[(h) as usize] -= 1_f32;
                        }
                        h = h.wrapping_add(1);
                    }
                }
                let ends: [f32; 2] = [
                    (unsafe {
                        let _delay: i32 = self.tank_[(0) as usize].d2.len;
                        x0x_Reverb_Line::Read(&self.tank_[(0) as usize].d2, _delay)
                    }),
                    (unsafe {
                        let _delay: i32 = self.tank_[(1) as usize].d2.len;
                        x0x_Reverb_Line::Read(&self.tank_[(1) as usize].d2, _delay)
                    }),
                ];
                {
                    let mut h: i32 = 0;
                    while ((h) < (2)) {
                        let t: *mut x0x_Reverb_Half = &mut self.tank_[(h) as usize];
                        let mut y: f32 = ((x) + ((self.decay_) * (ends[((1) - (h)) as usize])));
                        let m: f32 = (((*t).mod_ap.len as f32)
                            + ((swing) * (unsafe { FastSin2Pi_6(self.lfo_[(h) as usize]) })));
                        let d: f32 = (unsafe { x0x_Reverb_Line::ReadFrac(&(*t).mod_ap, m) });
                        let v: f32 = ((y) + ((0.7_f32) * (d)));
                        (unsafe { x0x_Reverb_Line::Write(&mut (*t).mod_ap, v) });
                        y = ((d) - ((0.7_f32) * (v)));
                        (unsafe { x0x_Reverb_Line::Write(&mut (*t).d1, y) });
                        y = (unsafe {
                            let _delay: i32 = (*t).d1.len;
                            x0x_Reverb_Line::Read(&(*t).d1, _delay)
                        });
                        (*t).lp += (((y) - ((*t).lp)) * (self.damp_));
                        y = (unsafe {
                            let _x: f32 = (((*t).lp) * (self.decay_));
                            x0x_Reverb_Line::Allpass(&mut (*t).ap, _x, -0.5_f32)
                        });
                        (unsafe { x0x_Reverb_Line::Write(&mut (*t).d2, y) });
                        h = h.wrapping_add(1);
                    }
                }
                let kk: f32 = self.tap_k_;
                let a: *const x0x_Reverb_Half = &self.tank_[(0) as usize];
                let b: *const x0x_Reverb_Half = &self.tank_[(1) as usize];
                let l: f32 =
                    (((((((unsafe { x0x_Reverb_Line::Tap(&(*b).d1, ((266_f32) * (kk))) })
                        + (unsafe { x0x_Reverb_Line::Tap(&(*b).d1, ((2974_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*b).ap, ((1913_f32) * (kk))) }))
                        + (unsafe { x0x_Reverb_Line::Tap(&(*b).d2, ((1996_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*a).d1, ((1990_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*a).ap, ((187_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*a).d2, ((1066_f32) * (kk))) }));
                let r: f32 =
                    (((((((unsafe { x0x_Reverb_Line::Tap(&(*a).d1, ((353_f32) * (kk))) })
                        + (unsafe { x0x_Reverb_Line::Tap(&(*a).d1, ((3627_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*a).ap, ((1228_f32) * (kk))) }))
                        + (unsafe { x0x_Reverb_Line::Tap(&(*a).d2, ((2673_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*b).d1, ((2111_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*b).ap, ((335_f32) * (kk))) }))
                        - (unsafe { x0x_Reverb_Line::Tap(&(*b).d2, ((121_f32) * (kk))) }));
                (*left.offset((i) as isize)) += ((0.5_f32) * (l));
                (*right.offset((i) as isize)) += ((0.5_f32) * (r));
                i = i.wrapping_add(1);
            }
        }
    }
}
impl Default for x0x_Reverb {
    fn default() -> Self {
        unsafe {
            x0x_Reverb {
                sr_: 48000_f32,
                k_: 1.6_f32,
                tap_k_: 1.6_f32,
                mem_: core::ptr::null_mut(),
                ok_: false,
                pre_: <x0x_Reverb_Line>::default(),
                in_ap_: [<x0x_Reverb_Line>::default(); 4],
                tank_: [<x0x_Reverb_Half>::default(); 2],
                size_: -1_f32,
                decay_: 0.5_f32,
                damp_: 0.5_f32,
                bw_: 0_f32,
                bw2_: 0_f32,
                bw_coef_: 0.5_f32,
                lfo_: [0_f32, 0.25_f32],
            }
        }
    }
}
const kNoTrigger_42: usize = !0_usize;
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_DrumFx_Settings {
    pub drive: f32,
    pub filter: f32,
    pub filter_res: f32,
    pub env_amount: f32,
    pub env_decay: f32,
    pub crush_bits: f32,
    pub crush_rate: f32,
}
impl Default for x0x_DrumFx_Settings {
    fn default() -> Self {
        unsafe {
            x0x_DrumFx_Settings {
                drive: 0_f32,
                filter: 0.5_f32,
                filter_res: 0_f32,
                env_amount: 0_f32,
                env_decay: 0.4_f32,
                crush_bits: 0_f32,
                crush_rate: 0_f32,
            }
        }
    }
}
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_DrumFx {
    sr_: f32,
    s_: x0x_DrumFx_Settings,
    f1_: f32,
    f2_: f32,
    held_: f32,
    pre_: f32,
    t1_: f32,
    t2_: f32,
    env_: f32,
    trig_depth_: f32,
    trig_at_: usize,
    hold_count_: i32,
}
impl x0x_DrumFx {
    pub unsafe fn Init(&mut self, mut sample_rate: f32) {
        self.sr_ = sample_rate;
    }
    pub unsafe fn Set(&mut self, s: *const x0x_DrumFx_Settings) {
        self.s_ = (*s);
    }
    pub unsafe fn Trigger(&mut self, mut at: usize, mut depth: f32) {
        self.trig_at_ = at;
        self.trig_depth_ = depth;
    }
    pub unsafe fn Process(&mut self, mut x: *mut f32, mut n: usize) {
        let driving: bool = ((self.s_.drive) > (0.005_f32));
        let gain: f32 = ((1_f32) + (((60_f32) * (self.s_.drive)) * (self.s_.drive)));
        let makeup: f32 = (unsafe { DrumDriveMakeup_12(self.s_.drive) });
        let pre_lp: f32 =
            (unsafe { TauToCoef_15(((1_f32) / (((2_f32) * kPi_0) * (1000_f32))), self.sr_) });
        let tone_lp: f32 = (unsafe {
            let _seconds: f32 =
                ((1_f32) / (((2_f32) * kPi_0) * ((8000_f32) - ((2000_f32) * (self.s_.drive)))));
            let _sr: f32 = self.sr_;
            TauToCoef_15(_seconds, _sr)
        });
        let f: f32 = ((self.s_.filter) - (0.5_f32));
        let lp: bool = ((f) < (-0.02_f32));
        let hp: bool = ((f) > (0.02_f32));
        let hz: f32 = if lp {
            ((200_f32) * (unsafe { FastExp2_3(((6.64_f32) * ((1_f32) + ((2_f32) * (f))))) }))
        } else {
            ((20_f32) * (unsafe { FastExp2_3((((7.97_f32) * (2_f32)) * (f))) }))
        };
        let g: f32 = (unsafe {
            libm::tanf(
                ((kPi_0 * (unsafe { Clamp_1(hz, 20_f32, ((0.45_f32) * (self.sr_))) }))
                    / (self.sr_)),
            )
        });
        let res: f32 = (unsafe { Clamp_1(self.s_.filter_res, 0_f32, 1_f32) });
        let kq: f32 = ((2_f32) * (unsafe { FastExp2_3(((-4.2_f32) * (res))) }));
        let mut a1: f32 = ((1_f32) / ((1_f32) + ((g) * ((g) + (kq)))));
        let mut a2: f32 = ((g) * (a1));
        let mut a3: f32 = ((g) * (a2));
        let trim: f32 = ((1_f32) / ((1_f32) + ((1.5_f32) * (res))));
        let env_oct: f32 = (((6_f32) * (unsafe { Clamp_1(self.s_.env_amount, 0_f32, 1_f32) }))
            * (if lp { 1_f32 } else { -1_f32 }));
        let env_k: f32 = (unsafe {
            let _seconds: f32 = ((0.01_f32)
                * (unsafe {
                    FastExp2_3(((6.64_f32) * (unsafe { Clamp_1(self.s_.env_decay, 0_f32, 1_f32) })))
                }));
            let _sr: f32 = self.sr_;
            TauToCoef_15(_seconds, _sr)
        });
        let env_on: bool = ((lp) || (hp)) && ((self.s_.env_amount) > (0.005_f32));
        let crush: bool = ((self.s_.crush_bits) > (0.005_f32));
        let levels: f32 = (unsafe { FastExp2_3(((15_f32) - ((12_f32) * (self.s_.crush_bits)))) });
        let hold: i32 = ((1) + ((((31_f32) * (self.s_.crush_rate)) * (self.s_.crush_rate)) as i32));
        {
            let mut i: usize = 0_usize;
            while ((i) < (n)) {
                if ((i) == (self.trig_at_))
                    || (((i) == ((n).wrapping_sub(1_usize))) && ((self.trig_at_) != kNoTrigger_42))
                {
                    {
                        self.env_ = self.trig_depth_;
                        self.trig_at_ = kNoTrigger_42
                    };
                }
                self.env_ -= ((self.env_) * (env_k));
                if (env_on) && (((i) & (3_usize)) == (0_usize)) {
                    let hz_e: f32 = (unsafe {
                        let _x: f32 = ((hz) * (unsafe { FastExp2_3(((env_oct) * (self.env_))) }));
                        let _hi: f32 = ((0.45_f32) * (self.sr_));
                        Clamp_1(_x, 20_f32, _hi)
                    });
                    let ge: f32 = (unsafe { libm::tanf(((kPi_0 * (hz_e)) / (self.sr_))) });
                    {
                        {
                            a1 = ((1_f32) / ((1_f32) + ((ge) * ((ge) + (kq)))));
                            a2 = ((ge) * (a1))
                        };
                        a3 = ((ge) * (a2))
                    };
                }
                let mut y: f32 = (*x.offset((i) as isize));
                if driving {
                    self.pre_ += (((y) - (self.pre_)) * (pre_lp));
                    let mut d: f32 = (((y) + ((y) - (self.pre_))) * (gain));
                    let a: f32 = (unsafe { libm::fabsf(d) });
                    if ((a) > (0.9_f32)) {
                        d = ((if ((d) < (0_f32)) { -1_f32 } else { 1_f32 })
                            * ((0.9_f32)
                                + ((0.1_f32)
                                    * (unsafe { FastTanh_10((((a) - (0.9_f32)) * (10_f32))) }))));
                    }
                    self.t1_ += (((d) - (self.t1_)) * (tone_lp));
                    self.t2_ += (((self.t1_) - (self.t2_)) * (tone_lp));
                    y = ((self.t2_) * (makeup));
                }
                if (lp) || (hp) {
                    let v3: f32 = ((y) - (self.f2_));
                    let v1: f32 = (((a1) * (self.f1_)) + ((a2) * (v3)));
                    let v2: f32 = (((self.f2_) + ((a2) * (self.f1_))) + ((a3) * (v3)));
                    {
                        self.f1_ = (((2_f32) * (v1)) - (self.f1_));
                        self.f2_ = (((2_f32) * (v2)) - (self.f2_))
                    };
                    y = ((if lp {
                        v2
                    } else {
                        (((y) - ((kq) * (v1))) - (v2))
                    }) * (trim));
                }
                if (crush) || ((hold) > (1)) {
                    let previous = self.hold_count_;
                    self.hold_count_ = previous.wrapping_sub(1);
                    if previous <= 0 {
                        self.hold_count_ = ((hold) - (1));
                        self.held_ = if crush {
                            ((unsafe {
                                libm::floorf(
                                    (((unsafe { Clamp_1(y, -1_f32, 1_f32) }) * (levels))
                                        + (0.5_f32)),
                                )
                            }) / (levels))
                        } else {
                            y
                        };
                    }
                    y = self.held_;
                }
                (*x.offset((i) as isize)) = y;
                i = i.wrapping_add(1);
            }
        }
    }
}
impl Default for x0x_DrumFx {
    fn default() -> Self {
        unsafe {
            x0x_DrumFx {
                sr_: 48000_f32,
                s_: <x0x_DrumFx_Settings>::default(),
                f1_: 0_f32,
                f2_: 0_f32,
                held_: 0_f32,
                pre_: 0_f32,
                t1_: 0_f32,
                t2_: 0_f32,
                env_: 0_f32,
                trig_depth_: 0_f32,
                trig_at_: kNoTrigger_42,
                hold_count_: 0,
            }
        }
    }
}
#[repr(C)]
#[derive(Copy, Clone)]
pub struct x0x_Compressor {
    amount_: f32,
    thresh_: f32,
    ratio_: f32,
    makeup_: f32,
    att_: f32,
    rel_: f32,
    gr_: f32,
}
impl x0x_Compressor {
    pub unsafe fn Init(&mut self, mut sample_rate: f32) {
        self.att_ = (unsafe { TauToCoef_15(0.008_f32, sample_rate) });
        self.rel_ = (unsafe { TauToCoef_15(0.12_f32, sample_rate) });
    }
    pub unsafe fn Set(&mut self, mut amount: f32) {
        self.amount_ = (unsafe { Clamp_1(amount, 0_f32, 1_f32) });
        self.thresh_ = ((-6_f32) - ((24_f32) * (self.amount_)));
        self.ratio_ = ((1.5_f32) + ((6.5_f32) * (self.amount_)));
        self.makeup_ = (((-self.thresh_) * ((1_f32) - ((1_f32) / (self.ratio_)))) * (0.45_f32));
    }
    pub unsafe fn Reduction(&self) -> f32 {
        return self.gr_;
    }
    pub unsafe fn Process(&mut self, mut left: *mut f32, mut right: *mut f32, mut n: usize) {
        if ((self.amount_) < (0.005_f32)) {
            self.gr_ = 0_f32;
            return;
        }
        {
            let mut i: usize = 0_usize;
            while ((i) < (n)) {
                let peak: f32 = (unsafe {
                    let ___x: f32 = (unsafe { libm::fabsf((*left.offset((i) as isize))) });
                    let ___y: f32 = (unsafe { libm::fabsf((*right.offset((i) as isize))) });
                    libm::fmaxf(___x, ___y)
                });
                let db: f32 = ((6.0206_f32) * (unsafe { FastLog2_8(((peak) + (1e-09_f32))) }));
                let over: f32 = ((db) - (self.thresh_));
                let mut want: f32 = 0_f32;
                if ((over) > (3_f32)) {
                    want = ((over) * ((1_f32) - ((1_f32) / (self.ratio_))));
                } else if ((over) > (-3_f32)) {
                    want = (((((1_f32) - ((1_f32) / (self.ratio_))) * ((over) + (3_f32)))
                        * ((over) + (3_f32)))
                        / (12_f32));
                }
                self.gr_ += (((want) - (self.gr_))
                    * (if ((want) > (self.gr_)) {
                        self.att_
                    } else {
                        self.rel_
                    }));
                let g: f32 =
                    (unsafe { FastExp2_3((((self.makeup_) - (self.gr_)) * (0.1660964_f32))) });
                (*left.offset((i) as isize)) *= g;
                (*right.offset((i) as isize)) *= g;
                i = i.wrapping_add(1);
            }
        }
    }
}
impl Default for x0x_Compressor {
    fn default() -> Self {
        unsafe {
            x0x_Compressor {
                amount_: 0_f32,
                thresh_: 0_f32,
                ratio_: 1_f32,
                makeup_: 0_f32,
                att_: 0.1_f32,
                rel_: 0.01_f32,
                gr_: 0_f32,
            }
        }
    }
}
