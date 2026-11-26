use super::{Message, pcm};
use wavegraph_core::{Port, Process, ProcessContext};
use wavegraph_sample::I24;

const KEY_BITS: u32 = 21;
const UNITY: i32 = 1 << 15;

/// A frequency-selective sidechain insert. Inlet 0 carries the program. Any
/// signal inlet can feed the detector, including inlet 0 for self-keying.
/// The key reaches full depth at 9/32 of full scale, starting at 1/32.
/// The low band takes the full gain reduction and the highs take half
/// the linear gain change. Full-depth attack and release take 2 and 90 ms.
#[derive(Clone, Copy, Debug)]
pub struct SidechainDucker<const INPUTS: usize = 2> {
    rate_hz: u32,
    key: usize,
    envelope: i32,
    attack_step: i32,
    release_step: i32,
    idle_gain: u8,
    max_reduction: u8,
    reduction: i32,
    low: i32,
    pole: i32,
    enabled: bool,
    blend: i32,
}

impl<const INPUTS: usize> SidechainDucker<INPUTS> {
    /// Gains are in units of 1/128. Returns `None` for a zero sample rate,
    /// missing external key inlet, or keyed gain above idle gain. Starts
    /// enabled with key inlet 1, full depth and a 160 Hz low shelf.
    #[must_use]
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The rate and times are positive."
    )]
    pub fn new(rate_hz: u32, keyed_gain: u8, idle_gain: u8) -> Option<Self> {
        let max_reduction = idle_gain.checked_sub(keyed_gain)?;
        if rate_hz == 0 || INPUTS < 2 {
            return None;
        }
        let step = |ms: u32| {
            i32::try_from((1000_u64 << KEY_BITS).div_ceil(u64::from(rate_hz) * u64::from(ms))).ok()
        };
        let mut insert = Self {
            rate_hz,
            key: 1,
            envelope: 0,
            attack_step: step(2)?,
            release_step: step(90)?,
            idle_gain,
            max_reduction,
            reduction: i32::from(max_reduction),
            low: 0,
            pole: 0,
            enabled: true,
            blend: 128,
        };
        insert.control(Message::SidechainHz(160));
        Some(insert)
    }

    #[allow(
        clippy::arithmetic_side_effects,
        reason = "The positive rate and bounded controls fit u64."
    )]
    fn control(&mut self, message: Message) {
        match message {
            Message::SidechainKey(key) if usize::from(key) < INPUTS => self.key = usize::from(key),
            Message::SidechainDepth(percent) => {
                self.reduction =
                    (i32::from(self.max_reduction) * i32::from(percent.min(100)) + 50) / 100;
            }
            Message::SidechainHz(hz) => {
                // Discretize an RC low-pass: pole = fs / (fs + 2πf).
                let rate = 1000 * u64::from(self.rate_hz);
                let denominator = rate + 6283 * u64::from(hz.clamp(40, 1000));
                self.pole = i32::try_from(((rate << 15) + denominator / 2) / denominator)
                    .expect("Q15 pole fits in i32");
            }
            Message::Enabled(on) => self.enabled = on,
            _ => {}
        }
    }

    fn render(&mut self, program: &[I24], key: &[I24], out: &mut [I24]) {
        let target_blend = if self.enabled { 128 } else { 0 };
        if self.low == 0
            && self.envelope == 0
            && self.blend == target_blend
            && program.iter().chain(key).all(|s| s.inner() == 0)
        {
            out.fill(I24::default());
            return;
        }
        for ((out, sample), key) in out.iter_mut().zip(program).zip(key) {
            *out = self.sample(*sample, *key, target_blend);
        }
    }

    /// Process a serial insert without copying its input to a scratch buffer.
    /// `None` keys from each original program sample; `Some` supplies an external
    /// key and must have the same length as `program`. Existing controls and the
    /// bypass crossfade apply exactly as they do through [`Process`].
    ///
    /// # Panics
    /// Panics if an external key has a different length from `program`.
    pub fn process_in_place(&mut self, program: &mut [I24], key: Option<&[I24]>) {
        if let Some(key) = key {
            assert_eq!(program.len(), key.len(), "sidechain key span");
        }
        let target_blend = if self.enabled { 128 } else { 0 };
        if self.low == 0
            && self.envelope == 0
            && self.blend == target_blend
            && program.iter().all(|s| s.inner() == 0)
            && key.is_none_or(|key| key.iter().all(|s| s.inner() == 0))
        {
            return;
        }
        match key {
            Some(key) => {
                for (sample, key) in program.iter_mut().zip(key) {
                    *sample = self.sample(*sample, *key, target_blend);
                }
            }
            None => {
                for sample in program {
                    *sample = self.sample(*sample, *sample, target_blend);
                }
            }
        }
    }

    #[inline(always)]
    #[allow(
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation,
        reason = "The bounded detector and byte gains limit the Q15 products and mixed Q23 result."
    )]
    fn sample(&mut self, sample: I24, key: I24, target_blend: i32) -> I24 {
        let x = sample.inner();
        let target = (key.inner().abs() - (1 << 18)).clamp(0, 1 << KEY_BITS);
        self.envelope += (target - self.envelope).clamp(-self.release_step, self.attack_step);
        // Truncate the residual toward zero so exact silence drains the filter.
        self.low = x + ((i64::from(self.low - x) * i64::from(self.pole)) / i64::from(UNITY)) as i32;
        let cut = (self.reduction * self.envelope) >> (KEY_BITS - 8);
        let high_cut = cut >> 1;
        let high_gain = (i32::from(self.idle_gain) << 8) - high_cut;
        let wet = ((i64::from(x) * i64::from(high_gain)
            - i64::from(self.low) * i64::from(cut - high_cut))
            >> 15) as i32;
        // Crossfade bypass over 64 samples while the detector keeps tracking.
        self.blend += (target_blend - self.blend).clamp(-2, 2);
        match self.blend {
            0 => sample,
            128 => pcm(wet),
            mix => pcm(x + ((i64::from(wet - x) * i64::from(mix)) >> 7) as i32),
        }
    }
}

impl<const INPUTS: usize> Process<Message, I24> for SidechainDucker<INPUTS> {
    fn inlets(&self) -> &[Port] {
        &[Port::Signal; INPUTS]
    }
    fn outlets(&self) -> &[Port] {
        &[Port::Signal]
    }

    fn process(&mut self, cx: &mut ProcessContext<'_, '_, Message, I24>) {
        let mut start = 0;
        for event in cx.events {
            let range = start..event.frame;
            self.render(
                &cx.inputs[0][range.clone()],
                &cx.inputs[self.key][range.clone()],
                &mut cx.outputs[0][range],
            );
            self.control(event.message);
            start = event.frame;
        }
        let range = start..cx.frames;
        self.render(
            &cx.inputs[0][range.clone()],
            &cx.inputs[self.key][range.clone()],
            &mut cx.outputs[0][range],
        );
    }
}
