//! A bounded interpreter for compiled, feed-forward signal graphs.
//!
//! The [wire format][wavegraph_core::bytecode] and [`Op`] are shared with host
//! compilers. Every instruction appends its outputs to the table of mono buses.
//!
//! Object indices name instances supplied by the application. Control messages
//! use the patch's Rust payload type and are delivered separately from this
//! instruction stream.

use crate::{Event, Port, Process, ProcessContext, dsp::Dsp};
use core::{fmt, marker::PhantomData};
use heapless::Vec;

pub use wavegraph_core::bytecode::{DecodeError, Op, VERSION};

/// Indicates which part of a program or render request could not be accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Missing or unsupported version byte.
    Version,
    /// Unknown opcode or incomplete operands.
    Instruction(DecodeError),
    /// The program exceeds an instruction, bus, or per-object port limit.
    Capacity,
    /// A source refers to a bus which has not yet been produced.
    Bus,
    /// An object is absent or called more than once per block.
    Object,
    /// The declared signal arity differs from the object's port layout.
    Ports,
    /// Too few signal buffers were supplied.
    Buffers,
    /// An input event has an invalid destination, frame offset, or order.
    Event,
}

/// A decoded, feed-forward block schedule. It can be prepared while another
/// program plays. `OPS` bounds the stored instructions. `PORTS` bounds the
/// signal inlets and outlets per object independently.
#[derive(Debug)]
pub struct Program<'code, const OPS: usize, const PORTS: usize = 8> {
    ops: Vec<Op<'code>, OPS>,
    buses: usize,
}

impl<'code, const OPS: usize, const PORTS: usize> Program<'code, OPS, PORTS> {
    /// Decode the wire format and validate its routing and resource bounds.
    ///
    /// # Errors
    /// Returns an error for invalid bytecode or exceeded capacities. Wire
    /// indices address up to 256 objects and 256 signal buses.
    pub fn decode(code: &'code [u8]) -> Result<Self, Error> {
        let Some((&VERSION, mut code)) = code.split_first() else {
            return Err(Error::Version);
        };
        let mut ops = Vec::new();
        let mut buses: usize = 0;
        let mut called = [false; 256];
        while !code.is_empty() {
            let (op, next) = Op::decode(code).map_err(Error::Instruction)?;
            code = next;
            let produced = match op {
                Op::Call {
                    object,
                    inputs,
                    outputs,
                } => {
                    if core::mem::replace(&mut called[usize::from(object)], true) {
                        return Err(Error::Object);
                    }
                    let outputs = usize::from(outputs);
                    if inputs.len() > PORTS || outputs > PORTS {
                        return Err(Error::Capacity);
                    }
                    if inputs.iter().any(|&bus| usize::from(bus) >= buses) {
                        return Err(Error::Bus);
                    }
                    outputs
                }
                Op::Gain { input, .. } => {
                    if usize::from(input) >= buses {
                        return Err(Error::Bus);
                    }
                    1
                }
                Op::Mix { left, right } => {
                    if usize::from(left.max(right)) >= buses {
                        return Err(Error::Bus);
                    }
                    1
                }
            };
            buses = buses.saturating_add(produced);
            if buses > 256 {
                return Err(Error::Capacity);
            }
            ops.push(op).map_err(|_| Error::Capacity)?;
        }
        Ok(Self { ops, buses })
    }

    /// Number of mono block buffers required by this program.
    #[must_use]
    pub fn buses(&self) -> usize {
        self.buses
    }

    /// Bind application-owned processor instances to the program's object indices.
    ///
    /// # Errors
    /// Returns [`Error::Object`] for missing instances and [`Error::Ports`] for
    /// signal layouts that disagree with the bytecode.
    pub fn bind<'run, 'node, M, S>(
        &'run self,
        objects: &'run mut [&'node mut dyn Process<M, S>],
    ) -> Result<Patch<'run, 'node, M, S, PORTS>, Error> {
        self.bind_nodes(objects)
    }

    /// Bind nodes while retaining their concrete type or trait-object bounds.
    /// For example, a table of `dyn Process<M, S> + Send` produces a `Send`
    /// patch that can be handed to an audio task.
    ///
    /// # Errors
    /// Returns the same object and port errors as [`Self::bind`].
    pub fn bind_nodes<'run, 'node, M, S, P: Process<M, S> + ?Sized>(
        &'run self,
        objects: &'run mut [&'node mut P],
    ) -> Result<Patch<'run, 'node, M, S, PORTS, P>, Error> {
        self.check_objects(objects)?;
        Ok(Patch {
            ops: &self.ops,
            buses: self.buses,
            objects,
            format: PhantomData,
        })
    }

    fn check_objects<M, S, P: Process<M, S> + ?Sized>(
        &self,
        objects: &[&mut P],
    ) -> Result<(), Error> {
        for op in &self.ops {
            if let Op::Call {
                object,
                inputs,
                outputs,
            } = op
            {
                let node = objects.get(usize::from(*object)).ok_or(Error::Object)?;
                let signals = |ports: &[Port]| ports.iter().filter(|&&p| p == Port::Signal).count();
                if inputs.len() != signals(node.inlets())
                    || usize::from(*outputs) != signals(node.outlets())
                {
                    return Err(Error::Ports);
                }
            }
        }
        Ok(())
    }
}

/// A decoded schedule bound to the stateful objects it will render.
pub struct Patch<'run, 'node, M, S, const PORTS: usize, P: ?Sized = dyn Process<M, S> + 'node> {
    ops: &'run [Op<'run>],
    buses: usize,
    objects: &'run mut [&'node mut P],
    // Messages and samples are borrowed only during render.
    format: PhantomData<fn(M, S)>,
}

impl<M, S, const PORTS: usize, P: ?Sized> fmt::Debug for Patch<'_, '_, M, S, PORTS, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Patch")
            .field("instructions", &self.ops.len())
            .field("buses", &self.buses)
            .finish_non_exhaustive()
    }
}

impl<'run, M, S, const PORTS: usize, P: Process<M, S> + ?Sized> Patch<'run, '_, M, S, PORTS, P> {
    /// Switch schedules between blocks, retaining every object's state.
    ///
    /// # Errors
    /// If the program needs more than `available_buses` or the object layouts
    /// are incompatible, the current schedule remains active.
    pub fn replace<const OPS: usize>(
        &mut self,
        program: &'run Program<'_, OPS, PORTS>,
        available_buses: usize,
    ) -> Result<(), Error> {
        if program.buses > available_buses {
            return Err(Error::Buffers);
        }
        program.check_objects(self.objects)?;
        self.ops = &program.ops;
        self.buses = program.buses;
        Ok(())
    }

    /// Render one block into caller-owned mono buses. The host chooses which
    /// bus to send to each physical output. `events[i]` belongs to object
    /// `i`; omitted trailing lists are empty. The `emit` sink receives the
    /// emitting object's index and its unmodified local outlet event. The
    /// host bounds the number of input events per block and the work in its
    /// event sink to fit its audio deadline.
    ///
    /// # Errors
    /// Invalid events, events addressed to inactive objects, and insufficient
    /// buffers are rejected before processing
    /// any object or writing any output.
    pub fn render<const FRAMES: usize>(
        &mut self,
        dsp: &mut impl Dsp<S>,
        buffers: &mut [[S; FRAMES]],
        events: &[&[Event<M>]],
        mut emit: impl FnMut(u8, Event<M>) -> Result<(), Event<M>>,
    ) -> Result<(), Error> {
        if buffers.len() < self.buses {
            return Err(Error::Buffers);
        }
        self.check_events::<FRAMES>(events)?;

        let mut used = 0;
        for op in self.ops {
            let (ready, free) = buffers.split_at_mut(used);
            let produced = match *op {
                Op::Call {
                    object,
                    inputs,
                    outputs,
                } => {
                    let inputs: Vec<_, PORTS> = inputs
                        .iter()
                        .map(|&bus| ready[usize::from(bus)].as_slice())
                        .collect();
                    let count = usize::from(outputs);
                    let mut outputs: Vec<_, PORTS> = free[..count]
                        .iter_mut()
                        .map(|bus| bus.as_mut_slice())
                        .collect();
                    let index = usize::from(object);
                    self.objects[index].process(&mut ProcessContext {
                        frames: FRAMES,
                        inputs: &inputs,
                        outputs: &mut outputs,
                        events: events.get(index).copied().unwrap_or_default(),
                        emit: &mut |event| emit(object, event),
                    });
                    count
                }
                Op::Gain { input, level } => {
                    dsp.gain(&ready[usize::from(input)], level, &mut free[0]);
                    1
                }
                Op::Mix { left, right } => {
                    dsp.mix(
                        &ready[usize::from(left)],
                        &ready[usize::from(right)],
                        &mut free[0],
                    );
                    1
                }
            };
            used = used.saturating_add(produced);
        }
        Ok(())
    }

    fn check_events<const FRAMES: usize>(&self, events: &[&[Event<M>]]) -> Result<(), Error> {
        if events.is_empty() {
            return Ok(());
        }
        if events.len() > self.objects.len() {
            return Err(Error::Event);
        }
        let mut addressed = events.iter().filter(|batch| !batch.is_empty()).count();
        for op in self.ops {
            let Op::Call { object, .. } = op else {
                continue;
            };
            let index = usize::from(*object);
            let Some(&batch) = events.get(index).filter(|batch| !batch.is_empty()) else {
                continue;
            };
            let mut previous = 0;
            let ports = self.objects[index].inlets().len();
            for event in batch {
                if event.frame < previous || event.frame >= FRAMES.max(1) || event.port >= ports {
                    return Err(Error::Event);
                }
                previous = event.frame;
            }
            addressed = addressed.saturating_sub(1);
        }
        // A live edit may have disconnected an object that still has queued notes.
        if addressed == 0 {
            Ok(())
        } else {
            Err(Error::Event)
        }
    }
}
