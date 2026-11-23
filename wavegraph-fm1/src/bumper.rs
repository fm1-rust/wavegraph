//! One-shot playback of the logo cue, mixed before the physical MASTER gain.

const PCM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bumper.s16le"));
const FRAMES: &[[u8; 2]] = {
    let (frames, tail) = PCM.as_chunks::<2>();
    assert!(tail.is_empty());
    frames
};

/// Remaining mono PCM belonging to the current splash.
#[derive(Debug, Default)]
pub struct Bumper {
    remaining: &'static [[u8; 2]],
}

impl Bumper {
    pub(super) fn start(&mut self) {
        self.remaining = FRAMES;
    }

    pub(super) fn stop(&mut self) {
        self.remaining = &[];
    }

    /// Reserve just the ready DMA half while locked. The audio task mixes it after unlocking.
    pub(super) fn take(&mut self, frames: usize, master: u16) -> &'static [[u8; 2]] {
        // MASTER is an unfiltered 10-bit ADC. Treat its bottom few counts as
        // off for the cue, including the initial zero before the first scan.
        if master <= 4 {
            self.stop();
        }
        let (block, tail) = self.remaining.split_at(frames.min(self.remaining.len()));
        self.remaining = tail;
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_logo_starts_playback_and_mute_cancels_it() {
        let mut cue = Bumper::default();
        assert!(cue.take(128, 1023).is_empty());
        for muted in [0, 1, 4] {
            cue.start();
            assert!(cue.take(128, muted).is_empty());
            assert!(cue.take(128, 1023).is_empty());
        }
        cue.start();
        assert_eq!(cue.take(128, 512), &FRAMES[..128]);
        assert!(cue.take(128, 0).is_empty());
        assert!(cue.take(128, 512).is_empty());
        cue.start();
        cue.stop();
        assert!(cue.take(128, 1023).is_empty());
    }

    #[test]
    fn a_partial_final_block_exhausts_the_cue() {
        let mut cue = Bumper::default();
        cue.start();
        for block in FRAMES.chunks(128) {
            assert_eq!(cue.take(128, 512), block);
        }
        assert!(cue.take(128, 512).is_empty());
    }
}
