//! FM-1 CPU0 output, serviced in the ALNK0 task.
use super::{Block, DeviceHost, Direction, Endpoint, Playback, Submission};
use core::convert::Infallible;
use fm1_audio::{MasterLevel, Output, Refill, RefillInterval};

/// Fixed board inventory. The playback DAC is the only PCM endpoint exposed by this BSP.
pub const DAC: Endpoint<(), &'static str> = Endpoint {
    id: (),
    name: "FM-1 DAC",
    direction: Direction::Output,
    channels: 2,
    rate: fm1_audio::FRAME_RATE,
};

/// The board's fixed audio inventory. Reading it never starts the peripheral.
#[derive(Clone, Copy, Debug, Default)]
pub struct Host;

impl DeviceHost for Host {
    type Id = ();
    type Name = &'static str;
    type Devices = [Endpoint<(), &'static str>; 1];
    type Error = Infallible;

    fn devices(&self) -> Result<Self::Devices, Infallible> {
        Ok([DAC])
    }
}

/// Borrow the existing output for a refill at the current panel MASTER setting.
#[derive(Debug)]
pub struct Dac<'a>(pub &'a mut Output, pub MasterLevel);

impl Playback for Dac<'_> {
    type Error = Infallible;

    type Timing = RefillInterval;

    fn render(
        &mut self,
        render: impl FnOnce(&mut Block, Self::Timing),
    ) -> Result<Submission, Infallible> {
        let result = self.0.render_with_interval(self.1, render);
        Ok(match result {
            Refill::WouldBlock => Submission::WouldBlock,
            Refill::Written => Submission::Written,
            Refill::Late => Submission::Late,
        })
    }
}

impl From<Submission> for Refill {
    fn from(value: Submission) -> Self {
        match value {
            Submission::WouldBlock => Self::WouldBlock,
            Submission::Written => Self::Written,
            Submission::Late => Self::Late,
        }
    }
}
