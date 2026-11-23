//! Factory audition pattern shared by the host example and the FM-1.
use crate::Voice::{self, *};

/// Two bars of straight sixteenths. Each slice lists the voices struck together.
/// The kick plays every quarter, with a snare and clap backbeat, offbeat hats and ride accents.
pub const DEMO_PATTERN: [&[Voice]; 32] = [
    &[Kick, ClosedHat, Ride],
    &[],
    &[OpenHat],
    &[ClosedHat],
    &[Kick, Snare, Clap],
    &[],
    &[OpenHat],
    &[ClosedHat],
    &[Kick, ClosedHat],
    &[],
    &[OpenHat, Ride],
    &[ClosedHat],
    &[Kick, Snare, Clap],
    &[],
    &[OpenHat],
    &[ClosedHat],
    &[Kick, ClosedHat, Ride],
    &[],
    &[OpenHat],
    &[ClosedHat],
    &[Kick, Snare, Clap],
    &[],
    &[OpenHat, Ride],
    &[ClosedHat],
    &[Kick, ClosedHat],
    &[],
    &[OpenHat],
    &[ClosedHat],
    &[Kick, Snare, Clap],
    &[ClosedHat],
    &[OpenHat, Ride],
    &[ClosedHat],
];
