//! macOS audio I/O through SDL3's CoreAudio driver. All methods run on the
//! application's control/render thread. No user callback is registered with SDL.
use crate::{Block, Capture, DeviceHost, Direction, Endpoint, FRAMES, Playback, Submission};
use core::{fmt, num::NonZeroU32};
use sdl3::{
    AudioSubsystem,
    audio::{AudioDevice, AudioDeviceID, AudioFormat, AudioSpec, AudioStreamOwner},
};
extern crate alloc;
use alloc::{string::String, vec, vec::Vec};
use std::io::{self, Read};

/// ID for this attachment of a physical device. Replugging may assign a fresh ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Id(AudioDeviceID);

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.id().0.fmt(f)
    }
}

/// Physical endpoints observed during a scan. Re-scan on the application's control timer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Devices {
    endpoints: Vec<Endpoint<Id, String>>,
    /// Current default recording device's display name, when available.
    pub default_input: Option<String>,
    /// Current default playback device's display name, when available.
    pub default_output: Option<String>,
}

impl Devices {
    /// Endpoints in stable instance-ID order, including duplicate display names.
    #[must_use]
    pub fn endpoints(&self) -> &[Endpoint<Id, String>] {
        &self.endpoints
    }
}

impl AsRef<[Endpoint<Id, String>]> for Devices {
    fn as_ref(&self) -> &[Endpoint<Id, String>] {
        self.endpoints()
    }
}

/// Native-endian stereo stream on the system default, or a channel selection on a specific device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    /// Follow the system default. SDL handles migration and mono/stereo conversion.
    Default,
    /// Pin to one device instance. A missing device or channel leaves the route offline.
    Device(Id, Channels),
}

/// Zero-based channel selection on a physical endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channels {
    /// Duplicate mono input to L/R or average L/R into the mono output.
    Mono(u16),
    /// Left channel of an adjacent stereo pair.
    Pair(u16),
}

impl Channels {
    fn indices(self, width: u16) -> Option<(usize, usize)> {
        let (l, r) = match self {
            Self::Mono(i) => (i, i),
            Self::Pair(i) => (i, i.checked_add(1)?),
        };
        (r < width).then_some((usize::from(l), usize::from(r)))
    }
}

/// Owns the SDL audio subsystem. Initialize on the main thread.
pub struct Host(AudioSubsystem);

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Host")
    }
}

impl Host {
    /// Initialize the macOS audio backend.
    ///
    /// # Errors
    /// Returns SDL initialization errors or an unexpected audio driver.
    pub fn new() -> io::Result<Self> {
        let audio = sdl3::init()
            .and_then(|sdl| sdl.audio())
            .map_err(io::Error::other)?;
        if audio.current_audio_driver() != "coreaudio" {
            return Err(io::Error::other("SDL did not select CoreAudio"));
        }
        Ok(Self(audio))
    }
}
impl DeviceHost for Host {
    type Id = Id;
    type Name = String;
    type Devices = Devices;
    type Error = io::Error;

    /// Enumerate physical recording and playback endpoints without opening a stream.
    /// An endpoint that vanishes during a scan can be picked up on the next scan.
    ///
    /// # Errors
    /// Returns a backend error if either device list cannot be obtained.
    fn devices(&self) -> io::Result<Devices> {
        let mut endpoints = Vec::new();
        for direction in [Direction::Input, Direction::Output] {
            let ids = match direction {
                Direction::Input => self.0.audio_recording_device_ids(),
                Direction::Output => self.0.audio_playback_device_ids(),
            }
            .map_err(io::Error::other)?;
            for id in ids {
                let device = AudioDevice::new(id, self.0.clone());
                let (Ok(name), Ok((spec, _))) = (device.name(), device.format()) else {
                    continue;
                };
                let Some(channels) = spec
                    .channels
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|&n| n > 0)
                else {
                    continue;
                };
                let Some(rate) = spec
                    .freq
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|&n| n > 0)
                else {
                    continue;
                };
                endpoints.push(Endpoint {
                    id: Id(id),
                    name,
                    direction,
                    channels,
                    rate: (rate, 1),
                });
            }
        }
        endpoints.sort_by_key(|e| e.id.0.id().0);
        Ok(Devices {
            endpoints,
            default_input: self.0.default_recording_device().name().ok(),
            default_output: self.0.default_playback_device().name().ok(),
        })
    }
}

/// Result of reconciling a selected route with the available devices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// A new stream is ready, with any old queued PCM discarded.
    Opened,
    /// The current stream can continue.
    Running,
    /// No compatible endpoint is available for the selected route.
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Layout {
    width: u16,
    left: usize,
    right: usize,
}

fn resolve(route: Route, direction: Direction, devices: &Devices) -> Option<Layout> {
    let (width, channels) = match route {
        Route::Default => {
            if !devices.endpoints().iter().any(|d| d.direction == direction) {
                return None;
            }
            (2, Channels::Pair(0))
        }
        Route::Device(id, channels) => {
            let device = devices
                .endpoints()
                .iter()
                .find(|e| e.id == id && e.direction == direction)?;
            (device.channels, channels)
        }
    };
    let (left, right) = channels.indices(width)?;
    Some(Layout { width, left, right })
}

// Each stream owns its SDL object and its byte staging buffer. No address into a
// WaveGraph block is installed in the backend or retained after a method returns.
struct Link {
    stream: AudioStreamOwner,
    layout: Layout,
    bytes: Vec<u8>,
    captured: usize,
    queue_limit: usize,
}

struct Port {
    rate: NonZeroU32,
    route: Option<Route>,
    link: Option<Link>,
}

impl Port {
    fn new(rate: NonZeroU32) -> Self {
        Self {
            rate,
            route: None,
            link: None,
        }
    }

    fn sync(
        &mut self,
        host: &Host,
        devices: &Devices,
        route: Route,
        direction: Direction,
    ) -> io::Result<Status> {
        let layout = resolve(route, direction, devices);
        let device = match (route, direction) {
            (Route::Default, Direction::Output) => host.0.default_playback_device(),
            (Route::Default, Direction::Input) => host.0.default_recording_device(),
            (Route::Device(id, _), _) => AudioDevice::new(id.0, host.0.clone()),
        };
        if self.route == Some(route)
            && let Some(link) = &mut self.link
            && Some(link.layout) == layout
        {
            link.queue_limit = queue_limit(&device, link.layout.width, self.rate);
            return Ok(Status::Running);
        }
        self.link = None;
        self.route = Some(route);
        let Some(layout) = layout else {
            return Ok(Status::Offline);
        };
        let spec = AudioSpec {
            freq: Some(i32::try_from(self.rate.get()).map_err(io::Error::other)?),
            channels: Some(i32::from(layout.width)),
            format: Some(AudioFormat::s32_sys()),
        };
        let probe = device.clone();
        let stream = device
            .open_device_stream(Some(&spec))
            .map_err(io::Error::other)?;
        if matches!(route, Route::Device(_, _)) {
            let (source, sink) = stream.get_format().map_err(io::Error::other)?;
            let device_side = match direction {
                Direction::Output => sink,
                Direction::Input => source,
            };
            if device_side.and_then(|spec| spec.channels) != Some(i32::from(layout.width)) {
                return Err(io::Error::other(
                    "device channel layout changed while opening route",
                ));
            }
        }
        let bytes = vec![0; block_bytes(layout.width)];
        stream.resume().map_err(io::Error::other)?;
        let queue_limit = queue_limit(&probe, layout.width, self.rate);
        self.link = Some(Link {
            stream,
            layout,
            bytes,
            captured: 0,
            queue_limit,
        });
        Ok(Status::Opened)
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "128 frames × 65535 channels × 4 fits in i32 on every supported host."
)]
fn block_bytes(channels: u16) -> usize {
    FRAMES * usize::from(channels) * size_of::<i32>()
}

// Keep two device periods in reserve, expressed in source bytes. SDL can migrate
// the default stream to a different clock or period while the graph keeps running.
fn queue_limit(device: &AudioDevice, width: u16, rate: NonZeroU32) -> usize {
    let period = device
        .format()
        .ok()
        .and_then(|(spec, frames)| {
            let hz = u64::try_from(spec.freq?)
                .ok()
                .and_then(core::num::NonZeroU64::new)?;
            Some(
                u64::try_from(frames?)
                    .ok()?
                    .saturating_mul(u64::from(rate.get()))
                    .div_ceil(hz.get()),
            )
        })
        .unwrap_or(512);
    let frames = period.saturating_mul(2).max(const { 4 * FRAMES as u64 });
    usize::try_from(frames.saturating_mul(u64::from(width)).saturating_mul(4)).unwrap_or(usize::MAX)
}

/// One application master routed to a Mac output.
pub struct Output(Port);

/// One independent capture stream. Drop it to stop recording.
pub struct Input(Port);

macro_rules! port {
    ($ty:ty, $direction:expr) => {
        impl $ty {
            /// Create an offline port at the graph's sample rate.
            #[must_use]
            pub fn new(rate: NonZeroU32) -> Self {
                Self(Port::new(rate))
            }

            /// Apply the desired route after a device scan, or retry a failed open.
            /// A changed explicit selection drops all staging and the old stream.
            ///
            /// # Errors
            /// Returns device-open, format or permission errors. The port stays offline.
            pub fn sync(
                &mut self,
                host: &Host,
                devices: &Devices,
                route: Route,
            ) -> io::Result<Status> {
                self.0.sync(host, devices, route, $direction)
            }
        }
        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($ty))
                    .field("route", &self.0.route)
                    .field("open", &self.0.link.is_some())
                    .finish()
            }
        }
    };
}
port!(Output, Direction::Output);
port!(Input, Direction::Input);

impl Playback for Output {
    type Error = io::Error;
    type Timing = ();

    fn render(&mut self, render: impl FnOnce(&mut Block, ())) -> io::Result<Submission> {
        let Some(link) = &mut self.0.link else {
            return Ok(Submission::WouldBlock);
        };
        let result = (|| {
            let queued = link.stream.queued_bytes().map_err(io::Error::other)?;
            // Keep room for the complete next block.
            if usize::try_from(queued).map_err(io::Error::other)?
                > link.queue_limit.saturating_sub(link.bytes.len())
            {
                return Ok(Submission::WouldBlock);
            }
            let mut block = [[0; 2]; FRAMES];
            render(&mut block, ());
            encode(&block, link.layout, &mut link.bytes);
            link.stream
                .put_data(&link.bytes)
                .map_err(io::Error::other)?;
            Ok(Submission::Written)
        })();
        if result.is_err() {
            self.0.link = None;
        }
        result
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "Channel indices are validated u16s, and their byte offsets fit usize."
)]
fn encode(block: &Block, layout: Layout, bytes: &mut [u8]) {
    fn word(sample: i32) -> [u8; 4] {
        (sample.clamp(-8_388_608, 8_388_607) << 8).to_ne_bytes()
    }
    for (out, &[l, r]) in bytes
        .chunks_exact_mut(usize::from(layout.width) * 4)
        .zip(block)
    {
        let a = layout.left * 4;
        let b = layout.right * 4;
        if a == b {
            let mono = l.midpoint(r);
            out[a..a + 4].copy_from_slice(&word(mono));
        } else {
            out[a..a + 4].copy_from_slice(&word(l));
            out[b..b + 4].copy_from_slice(&word(r));
        }
    }
}

impl Capture for Input {
    type Error = io::Error;

    fn read(&mut self, out: &mut Block) -> io::Result<bool> {
        let Some(link) = &mut self.0.link else {
            return Ok(false);
        };
        match link.stream.read(&mut link.bytes[link.captured..]) {
            Ok(n) => link.captured = link.captured.saturating_add(n),
            Err(e) => {
                self.0.link = None;
                return Err(e);
            }
        }
        if link.captured < link.bytes.len() {
            return Ok(false);
        }
        decode(&link.bytes, link.layout, out);
        link.captured = 0;
        Ok(true)
    }
}

#[allow(
    clippy::arithmetic_side_effects,
    reason = "The validated channel offsets fit inside each complete interleaved frame."
)]
fn decode(bytes: &[u8], layout: Layout, block: &mut Block) {
    for (frame, dest) in bytes.chunks_exact(usize::from(layout.width) * 4).zip(block) {
        *dest = [layout.left, layout.right].map(|channel| {
            let i = channel * 4;
            i32::from_ne_bytes([frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]) >> 8
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;
    use std::time::Instant;

    #[test]
    fn route_q23_on_a_four_channel_interface() {
        let layout = Layout {
            width: 4,
            left: 2,
            right: 3,
        };
        let mut bytes = vec![0; block_bytes(4)];
        let mut decoded = [[0; 2]; FRAMES];
        let mut source = [[i32::MIN, i32::MAX]; FRAMES];
        source[1] = [-1, 1];
        encode(&source, layout, &mut bytes);
        assert_eq!(&bytes[..8], &[0; 8]);
        decode(&bytes, layout, &mut decoded);
        assert_eq!(decoded[0], [-8_388_608, 8_388_607]);
        assert_eq!(decoded[1], [-1, 1]);
        let mono = Layout {
            width: 4,
            left: 1,
            right: 1,
        };
        source.fill([600, -200]);
        encode(&source, mono, &mut bytes);
        decode(&bytes, mono, &mut decoded);
        assert_eq!(decoded, [[200; 2]; FRAMES]);
        assert_eq!(Channels::Pair(3).indices(4), None);
        assert_eq!(Channels::Pair(u16::MAX).indices(4), None);
        assert_eq!(Channels::Mono(0).indices(0), None);
    }

    // Run in its own test process with SDL_AUDIO_DRIVER=dummy. No microphone permission.
    #[test]
    #[ignore = "requires SDL's dummy driver"]
    fn duplex_and_route_removal() -> io::Result<()> {
        let sdl = sdl3::init().map_err(io::Error::other)?;
        let host = Host(sdl.audio().map_err(io::Error::other)?);
        assert_eq!(host.0.current_audio_driver(), "dummy");
        let devices = host.devices()?;
        let output_id = devices
            .endpoints()
            .iter()
            .find(|e| e.direction == Direction::Output)
            .expect("dummy output")
            .id;
        let route = Route::Device(output_id, Channels::Mono(0));
        let rate = NonZeroU32::new(48_000).expect("positive");
        let mut output = Output::new(rate);
        let mut input = Input::new(rate);
        let missing = Devices::default();
        assert_eq!(input.sync(&host, &devices, route)?, Status::Offline);
        assert_eq!(output.sync(&host, &devices, route)?, Status::Opened);
        assert_eq!(output.sync(&host, &devices, route)?, Status::Running);
        // Pausing the consumer makes queue pressure deterministic.
        output
            .0
            .link
            .as_ref()
            .expect("open")
            .stream
            .pause()
            .map_err(io::Error::other)?;
        let mut rendered = 0usize;
        let mut writes = 0usize;
        for _ in 0..2048 {
            let result = output.render(|block, ()| {
                assert!(block.iter().all(|&f| f == [0; 2]));
                block.fill([4096; 2]);
                rendered = rendered.saturating_add(1);
            })?;
            if result == Submission::WouldBlock {
                break;
            }
            assert_eq!(result, Submission::Written);
            writes = writes.saturating_add(1);
        }
        assert!(writes > 0 && writes < 2048);
        assert_eq!(writes, rendered);
        assert_eq!(output.sync(&host, &missing, route)?, Status::Offline);
        assert_eq!(
            output.render(|_, ()| unreachable!())?,
            Submission::WouldBlock
        );
        assert_eq!(
            output.sync(&host, &devices, Route::Default)?,
            Status::Opened
        );
        assert_eq!(input.sync(&host, &devices, Route::Default)?, Status::Opened);
        let started = Instant::now();
        let mut captured = [[42; 2]; FRAMES];
        while !input.read(&mut captured)? {
            assert_eq!(captured, [[42; 2]; FRAMES]);
            assert!(started.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(captured, [[0; 2]; FRAMES]);
        Ok(())
    }
}
