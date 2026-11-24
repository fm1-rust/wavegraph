#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod tuning;

pub mod bytecode;
pub mod midi;
pub mod polyphony;
pub use bytecode::Op;
pub use tuning::Tuning;

/// A target can implement these operations with its own DSP kernels. Each
/// slice has the same frame count and the output is distinct from the inputs.
/// Calls must finish within the audio deadline and fully overwrite the output.
pub trait Dsp<S> {
    /// Scale by `level / 128` and saturate to the sample format's nominal range.
    /// For fixed-point PCM, round toward negative infinity.
    fn gain(&mut self, input: &[S], level: u8, output: &mut [S]);

    /// Add two signals and saturate to the nominal sample range.
    fn mix(&mut self, left: &[S], right: &[S], output: &mut [S]);
}

/// A patch object. The patch selects a common message type `M` and signal
/// sample type `S`. Each object owns its state between calls.
pub trait Process<M = (), S = f32> {
    /// Inlets in left-to-right patch order. The layout stays fixed while connected.
    fn inlets(&self) -> &[Port];

    /// Outlets in left-to-right patch order. The layout stays fixed while connected.
    fn outlets(&self) -> &[Port];

    /// Process the supplied signals and messages. On the audio thread this
    /// work must be bounded, allocation-free and nonblocking.
    fn process(&mut self, cx: &mut ProcessContext<'_, '_, M, S>);
}

/// Kind of connection exposed by an inlet or outlet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Port {
    /// Carries discrete messages such as a bang, note or parameter change.
    Message,
    /// Carries a block of samples. A signal inlet can also receive messages
    /// when the object defines a meaning for them.
    Signal,
}

/// A control message at a frame offset within a processing call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event<M> {
    /// Apply the message immediately before this zero-based frame.
    pub frame: usize,
    /// Local inlet number on delivery, or local outlet number on emission.
    pub port: usize,
    /// The patch's message payload.
    pub message: M,
}

/// Borrowed signals and messages for one call. The output table can be
/// reborrowed independently of its sample storage.
#[allow(
    missing_debug_implementations,
    reason = "The event sink is an opaque callback."
)]
pub struct ProcessContext<'call, 'output, M = (), S = f32> {
    /// Number of frames in every signal buffer. A zero-frame call delivers
    /// messages at offset zero without advancing audio time.
    pub frames: usize,
    /// One shared buffer per signal inlet, in inlet order. For inlets
    /// `[Message, Signal]`, buffer zero belongs to inlet one.
    pub inputs: &'call [&'call [S]],
    /// One exclusive buffer per signal outlet, in outlet order. The processor
    /// overwrites every frame. An additive renderer clears its output first.
    pub outputs: &'call mut [&'output mut [S]],
    /// Messages in nondecreasing frame order. Apply ties in slice order.
    /// In a nonempty block, offsets are less than `frames`. An event at the
    /// end belongs to the next block.
    pub events: &'call [Event<M>],
    /// Emit in frame order on a message outlet. The host maps the outlet to
    /// downstream inlets and preserves the frame offset and emission order.
    /// Return the original event if the host cannot accept it.
    pub emit: &'call mut dyn FnMut(Event<M>) -> Result<(), Event<M>>,
}
