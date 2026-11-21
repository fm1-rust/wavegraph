//! Portable instructions shared by patch compilers and interpreters.
//!
//! A program starts with [`VERSION`] followed by instructions. Every
//! instruction appends its outputs to a table of mono signal buffers (buses).
//! Sources refer to earlier buses. All fields occupy one byte.
//!
//! | Opcode | Operands | Outputs |
//! | --- | --- | --- |
//! | 1 (call) | object index, signal input count, signal output count, one source bus per signal input | One bus per signal outlet, in order |
//! | 2 (gain) | source bus, unsigned gain (128 is unity) | One bus |
//! | 3 (mix) | left bus, right bus | One bus |

/// Version byte that precedes a stream of encoded [`Op`]s.
pub const VERSION: u8 = 1;

const CALL: u8 = 1;
const GAIN: u8 = 2;
const MIX: u8 = 3;

/// A block-level signal graph instruction. Object IDs refer to stateful native
/// instances installed by the application. A decoded call borrows its input
/// bus indices directly from the bytecode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op<'a> {
    /// Run a patch object once on the current audio block.
    Call {
        /// Index in the patch's object table.
        object: u8,
        /// One source bus per signal inlet, in inlet order.
        inputs: &'a [u8],
        /// Number of consecutive buses produced, in signal outlet order.
        outputs: u8,
    },
    /// Scale one input bus and append the saturated result.
    Gain {
        /// Source bus.
        input: u8,
        /// Gain in units of 1/128. A value of 128 is unity.
        level: u8,
    },
    /// Append the saturating sum of two input buses.
    Mix {
        /// First source bus.
        left: u8,
        /// Second source bus.
        right: u8,
    },
}

/// Errors decoding a single instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// An opcode or one of its operands is missing.
    Truncated,
    /// This version of the instruction set has no such opcode.
    UnknownOpcode(u8),
}

/// Errors encoding a single instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// The destination is too short for the entire instruction.
    BufferTooSmall,
    /// The input list length exceeds its one-byte wire field.
    TooManyInputs,
}

impl<'a> Op<'a> {
    /// Decode the first instruction and return the unconsumed suffix.
    ///
    /// # Errors
    /// Returns [`DecodeError`] for an incomplete or unrecognized instruction.
    pub fn decode(bytes: &'a [u8]) -> Result<(Self, &'a [u8]), DecodeError> {
        match bytes {
            [CALL, object, count, outputs, rest @ ..] => {
                let (inputs, rest) = rest
                    .split_at_checked(usize::from(*count))
                    .ok_or(DecodeError::Truncated)?;
                Ok((
                    Self::Call {
                        object: *object,
                        inputs,
                        outputs: *outputs,
                    },
                    rest,
                ))
            }
            [GAIN, input, level, rest @ ..] => Ok((
                Self::Gain {
                    input: *input,
                    level: *level,
                },
                rest,
            )),
            [MIX, left, right, rest @ ..] => Ok((
                Self::Mix {
                    left: *left,
                    right: *right,
                },
                rest,
            )),
            [] | [CALL | GAIN | MIX, ..] => Err(DecodeError::Truncated),
            [unknown, ..] => Err(DecodeError::UnknownOpcode(*unknown)),
        }
    }

    /// Write one complete instruction and return the number of bytes written.
    /// The program's leading version byte is supplied by the caller.
    ///
    /// # Errors
    /// Returns [`EncodeError`] before writing anything if the instruction
    /// cannot be represented or the output buffer is too short.
    pub fn encode(self, output: &mut [u8]) -> Result<usize, EncodeError> {
        match self {
            Self::Call {
                object,
                inputs,
                outputs,
            } => {
                let count = u8::try_from(inputs.len()).map_err(|_| EncodeError::TooManyInputs)?;
                let len = 4 + usize::from(count);
                let dst = output.get_mut(..len).ok_or(EncodeError::BufferTooSmall)?;
                dst[..4].copy_from_slice(&[CALL, object, count, outputs]);
                dst[4..].copy_from_slice(inputs);
                Ok(len)
            }
            Self::Gain { input, level } => write_three(output, [GAIN, input, level]),
            Self::Mix { left, right } => write_three(output, [MIX, left, right]),
        }
    }
}

fn write_three(output: &mut [u8], bytes: [u8; 3]) -> Result<usize, EncodeError> {
    output
        .get_mut(..3)
        .ok_or(EncodeError::BufferTooSmall)?
        .copy_from_slice(&bytes);
    Ok(3)
}
