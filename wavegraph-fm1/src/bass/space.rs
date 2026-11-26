//! Parallel bass, filtered ping-pong echo and a stereo plate for the Pump Panel patch.

use wavegraph_303::{Reverb, StereoFrame};
use wavegraph_sample::I24;

const DIVISOR: usize = 5;
pub(crate) const CHUNK: usize = 64;
// At 44,118 Hz the reduced-rate Dattorro tank at size 0.38 fits in 7,424 samples.
pub(crate) const PLATE_SAMPLES: usize = 7_424;

/// Two eighth notes at the lowest supported tempo and one guard sample.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "At one fifth rate, two seconds fit in a 32-bit usize"
)]
pub(crate) const fn echo_samples(rate: u32) -> usize {
    2 * (rate as usize).div_ceil(DIVISOR) + 1
}

/// Combine the pedal with the clean post-VCA bass. The pedal inverts the low band.
#[derive(Default)]
pub(crate) struct Tone {
    low: f32,
    coefficient: f32,
}

#[allow(clippy::arithmetic_side_effects)]
impl Tone {
    pub(crate) fn new(rate: f32) -> Self {
        Self {
            low: 0.0,
            coefficient: 1.0 - micromath::F32Ext::exp(-core::f32::consts::TAU * 180.0 / rate),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.low = 0.0;
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    // SAFETY: FM-1 startup copies the voice blend into executable SRAM.
    #[cfg_attr(
        target_os = "none",
        unsafe(link_section = ".data.wavegraph_audio"),
        inline(never)
    )]
    pub(crate) fn mix(&mut self, driven: &mut [I24], clean: &mut [I24], drive: f32) {
        let blend = (drive * 1.5).min(1.0);
        // The square-wave take used an extra 1.63 dB of trim on the voice and both sends.
        let gain = 1.0 - (1.0 - 0.6302) * blend;
        for (direct, send) in driven.iter_mut().zip(clean) {
            self.low += self.coefficient * (send.inner() as f32 - self.low);
            let pedal = direct.inner() as f32 * gain;
            *direct = pcm((pedal - 7.5 * blend * gain * self.low) as i32);
            *send = pcm(pedal as i32);
        }
    }
}

fn pcm(sample: i32) -> I24 {
    I24::new_unchecked(sample.clamp(-8_388_608, 8_388_607))
}

/// One Butterworth section. The low pass precedes decimation and the high pass
/// keeps the centered bass out of the reflections.
struct Filter {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

#[allow(clippy::arithmetic_side_effects)]
impl Filter {
    fn new(rate: f32, hz: f32, high_pass: bool) -> Self {
        let k = micromath::F32Ext::tan(core::f32::consts::PI * (hz / rate).min(0.40));
        let k2 = k * k;
        let norm = 1.0 / (1.0 + core::f32::consts::SQRT_2 * k + k2);
        let b = if high_pass {
            [norm, -2.0 * norm, norm]
        } else {
            [k2 * norm, 2.0 * k2 * norm, k2 * norm]
        };
        Self {
            b,
            a: [
                2.0 * (k2 - 1.0) * norm,
                (1.0 - core::f32::consts::SQRT_2 * k + k2) * norm,
            ],
            z: [0.0; 2],
        }
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z = [
            self.b[1] * x - self.a[0] * y + self.z[1],
            self.b[2] * x - self.a[1] * y,
        ];
        y
    }
}

/// One shared linear return. Only the pedal taps of selected strips feed it.
pub(crate) struct Space<'a> {
    rate: u32,
    plate: Reverb<'a>,
    echo: &'a mut [i16],
    write: usize,
    delay: usize,
    next_delay: usize,
    target: usize,
    fade: u8,
    damping: f32,
    damped: [f32; 2],
    low_pass: Filter,
    high_pass: Filter,
    phase: usize,
    sum: f32,
    wet: [f32; 2],
    increment: [f32; 2],
    tail: usize,
    tail_length: usize,
}

#[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
impl<'a> Space<'a> {
    pub(crate) fn new(rate: u32, echo: &'a mut [i16], storage: &'a mut [f32]) -> Option<Self> {
        if rate == 0 || echo.len() < echo_samples(rate) {
            return None;
        }
        let reduced = rate as f32 / DIVISOR as f32;
        let plate = Reverb::with_max_size(reduced, storage, 0.38)?;
        echo.fill(0);
        let mut space = Self {
            rate,
            plate,
            echo,
            write: 0,
            delay: 1,
            next_delay: 1,
            target: 1,
            fade: 0,
            damping: 1.0 - micromath::F32Ext::exp(-core::f32::consts::TAU * 3700.0 / reduced),
            damped: [0.0; 2],
            low_pass: Filter::new(rate as f32, 3800.0, false),
            high_pass: Filter::new(reduced, 250.0, true),
            phase: 0,
            sum: 0.0,
            wet: [0.0; 2],
            increment: [0.0; 2],
            tail: 0,
            tail_length: 0,
        };
        space.set_tempo(122);
        Some(space)
    }

    pub(crate) fn set_tempo(&mut self, bpm: u16) {
        let divisor = usize::from(bpm.clamp(30, 300)) * DIVISOR;
        let target = ((self.rate as usize * 30 + divisor / 2) / divisor).max(1);
        if self.target == target && self.tail_length != 0 {
            return;
        }
        self.target = target;
        if self.tail == 0 {
            self.delay = self.target;
            self.next_delay = self.target;
            self.fade = 0;
        }
        self.tail_length = (self.rate as usize * 3).max(self.target * DIVISOR * 9);
        if self.tail != 0 {
            self.tail = self.tail.max(self.tail_length);
        }
    }

    pub(crate) fn ringing(&self) -> bool {
        self.tail != 0
    }

    fn tap(&self, age: usize) -> f32 {
        let index = if self.write >= age {
            self.write - age
        } else {
            self.echo.len() - (age - self.write)
        };
        f32::from(self.echo[index]) * 256.0
    }

    #[allow(clippy::cast_possible_truncation)]
    fn echo(&mut self, send: f32) -> StereoFrame {
        if self.fade == 0 && self.delay != self.target {
            self.next_delay = self.target;
            self.fade = 64;
        }
        let mut odd = self.tap(self.delay);
        let mut even = self.tap(2 * self.delay);
        if self.fade != 0 {
            let mix = f32::from(65 - self.fade) / 64.0;
            odd += (self.tap(self.next_delay) - odd) * mix;
            even += (self.tap(2 * self.next_delay) - even) * mix;
            self.fade -= 1;
            if self.fade == 0 {
                self.delay = self.next_delay;
            }
        }
        // A single ring spanning two eighth notes carries the alternating left and right taps.
        // Two successive damping passes and feedback squared complete a stereo round trip.
        self.damped[0] += self.damping * (even - self.damped[0]);
        self.damped[1] += self.damping * (self.damped[0] - self.damped[1]);
        self.echo[self.write] = ((send + 0.27 * 0.27 * self.damped[1]) / 256.0) as i16;
        self.write += 1;
        if self.write == self.echo.len() {
            self.write = 0;
        }
        let second = 0.27 * self.damped[0];
        StereoFrame {
            left: 0.39 * odd + 0.13 * second,
            right: 0.13 * odd + 0.39 * second,
        }
    }

    // Batch each low-rate part of a block so the alternating echo and plate
    // kernels stay in the instruction cache. Preserve both decimator phases
    // across DMA boundaries, including the partial group at the end.
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: FM-1 startup copies the bounded stereo kernel into executable SRAM.
    #[cfg_attr(
        target_os = "none",
        unsafe(link_section = ".data.wavegraph_audio"),
        inline(never)
    )]
    pub(crate) fn render(&mut self, input: &[f32], output: &mut [[i32; 2]]) {
        assert_eq!(input.len(), output.len());
        debug_assert!(input.len() <= CHUNK);
        let last_input = input.iter().rposition(|&x| x != 0.0);
        if let Some(last) = last_input {
            self.tail = self.tail_length.saturating_sub(input.len() - 1 - last);
        } else if self.tail == 0 {
            self.phase = (self.phase + input.len()) % DIVISOR;
            output.fill([0; 2]);
            return;
        } else {
            self.tail = self.tail.saturating_sub(input.len());
        }
        const TICKS: usize = CHUNK.div_ceil(DIVISOR);
        let mut feed = [0.0; TICKS];
        let mut wet = [StereoFrame::default(); TICKS];
        let start = self.phase;
        let mut count = 0;
        // After leaving the patch the echo and plate still ring. Once both
        // input filters have settled below 1e-12 of one Q23 count, only
        // the reduced-rate clocks need to advance until the next input.
        if last_input.is_none()
            && self.sum.abs() < 1e-12
            && self
                .low_pass
                .z
                .iter()
                .chain(&self.high_pass.z)
                .all(|v| v.abs() < 1e-12)
        {
            self.sum = 0.0;
            self.low_pass.z = [0.0; 2];
            self.high_pass.z = [0.0; 2];
            count = (start + input.len()) / DIVISOR;
            self.phase = (start + input.len()) % DIVISOR;
            for frame in &mut wet[..count] {
                *frame = self.echo(0.0);
            }
        } else {
            for &sample in input {
                self.sum += self.low_pass.tick(sample);
                self.phase += 1;
                if self.phase == DIVISOR {
                    self.phase = 0;
                    let band = self.high_pass.tick(self.sum / DIVISOR as f32);
                    self.sum = 0.0;
                    wet[count] = self.echo(band);
                    feed[count] = 0.373 * band;
                    count += 1;
                }
            }
        }
        self.plate
            .process_add(&feed[..count], &mut wet[..count])
            .expect("matching plate spans");
        let mut phase = start;
        let mut targets = wet[..count].iter();
        for frame in output {
            phase += 1;
            if phase == DIVISOR {
                phase = 0;
                let target = targets.next().expect("one reflection per completed group");
                self.increment = [
                    (target.left - self.wet[0]) / DIVISOR as f32,
                    (target.right - self.wet[1]) / DIVISOR as f32,
                ];
            }
            self.wet[0] += self.increment[0];
            self.wet[1] += self.increment[1];
            *frame = self.wet.map(|sample| sample as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_tail_and_next_attack_survive_uneven_block_boundaries() {
        fn render(chunks: &[usize]) -> Vec<[i32; 2]> {
            let rate = 44_118;
            let mut echo = vec![0; echo_samples(rate)];
            let mut plate = vec![0.0; PLATE_SAMPLES];
            let mut space = Space::new(rate, &mut echo, &mut plate).expect("factory stereo return");
            space.set_tempo(144);
            let mut input = vec![0.0; 23_000];
            input[2] = 2_097_152.0;
            input[14_101] = -1_048_576.0;
            let mut result = vec![[0; 2]; input.len()];
            let mut pos = 0_usize;
            for &n in chunks.iter().cycle() {
                let end = pos.saturating_add(n).min(input.len());
                space.render(&input[pos..end], &mut result[pos..end]);
                pos = end;
                if pos == input.len() {
                    break;
                }
            }
            result
        }
        let expected = render(&[64]);
        assert!(expected[9200..14_000].iter().any(|f| f[0] != f[1]));
        assert_eq!(render(&[1, 13, 31, 64]), expected);
    }

    #[test]
    #[allow(clippy::arithmetic_side_effects, clippy::cast_precision_loss)]
    fn echo_follows_the_eighth_and_alternates() {
        const RATE: u32 = 44_100;
        for bpm in [30, 144, 300] {
            let mut history = vec![0; echo_samples(RATE)];
            let mut room = vec![0.0; PLATE_SAMPLES];
            let mut space = Space::new(RATE, &mut history, &mut room).expect("factory space");
            space.set_tempo(bpm);
            let delay = (RATE as usize * 30 + usize::from(bpm) * DIVISOR / 2)
                / (usize::from(bpm) * DIVISOR);
            let mut amplitudes = [[0.0; 2]; 3];
            for t in 0..=(3 * delay) {
                let impulse = if t == 0 { 1_048_576.0 } else { 0.0 };
                let wet = space.echo(impulse);
                let echo = [wet.left, wet.right];
                if t < delay {
                    assert!(echo.iter().all(|x| x.abs() < 1.0));
                }
                if t != 0 && t.is_multiple_of(delay) {
                    amplitudes[t / delay - 1] = echo;
                }
            }
            assert!((amplitudes[0][0] / 1_048_576.0 - 0.39).abs() < 0.001);
            assert!((amplitudes[0][1] / amplitudes[0][0] - 1.0 / 3.0).abs() < 0.001);
            assert!((amplitudes[1][0] / amplitudes[1][1] - 1.0 / 3.0).abs() < 0.001);
            assert!((0.22..0.28).contains(&(amplitudes[1][1] / amplitudes[0][0])));
            assert!(amplitudes[2][0] > amplitudes[2][1]);
        }
    }
}
