#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(all(feature = "coreaudio", target_os = "macos"))]
extern crate std;

#[cfg(all(feature = "coreaudio", target_os = "macos"))]
pub mod coreaudio;
#[cfg(feature = "fm1")]
pub mod fm1;

/// Frames per graph render and FM-1 DAC half.
pub const FRAMES: usize = 128;
/// Stereo Q23 samples in i32 accumulators, with headroom until the output boundary.
pub type Block = [[i32; 2]; FRAMES];

/// Role of a hardware endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// Deliver captured samples to the application.
    Input,
    /// Play samples produced by the application.
    Output,
}

/// An enumerated physical endpoint. Names are for display. Select by identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint<Id, Name> {
    /// Backend identity of this device instance.
    pub id: Id,
    /// Display name, possibly shared with another device.
    pub name: Name,
    /// Signal flow.
    pub direction: Direction,
    /// Channels exposed by the backend.
    pub channels: u16,
    /// Backend-reported frame rate in frames per second, numerator and denominator.
    pub rate: (u32, u32),
}

/// Enumerate an owned snapshot on the control path. Snapshots can outlive a device.
pub trait DeviceHost {
    /// Device identity, suitable for distinguishing duplicate display names.
    type Id: Copy + Eq;
    /// Display label.
    type Name: AsRef<str>;
    /// Storage chosen by the backend, such as a fixed array or a desktop catalog.
    type Devices: AsRef<[Endpoint<Self::Id, Self::Name>]>;
    /// Failure to obtain an inventory.
    type Error;
    /// Observe the currently available inputs and outputs.
    ///
    /// # Errors
    /// Returns backend enumeration errors.
    fn devices(&self) -> Result<Self::Devices, Self::Error>;
}

/// Outcome of a single refill attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Submission {
    /// Backend accepted the rendered block. Queued playback can still be interrupted.
    Written,
    /// Hardware deadline passed during this render.
    Late,
    /// No space or no open output. The renderer did not run.
    WouldBlock,
}

/// Sink for a stereo graph master. Service it on the backend's owning thread.
pub trait Playback {
    /// Backend error.
    type Error;
    /// Evidence available before selecting samples. FM-1 supplies its measured DAC interval.
    type Timing;
    /// Call the renderer at most once, with a cleared block. On submission failure
    /// after rendering, the caller's graph may already have advanced.
    ///
    /// # Errors
    /// Returns transport errors from the backend.
    fn render(
        &mut self,
        render: impl FnOnce(&mut Block, Self::Timing),
    ) -> Result<Submission, Self::Error>;
}

/// Capture one block from an input route.
pub trait Capture {
    /// Backend error.
    type Error;
    /// Return true for a full block. On false, the destination retains its contents.
    ///
    /// # Errors
    /// Returns transport errors from the backend.
    fn read(&mut self, out: &mut Block) -> Result<bool, Self::Error>;
}
