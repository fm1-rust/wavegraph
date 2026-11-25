//! A one-node WaveGraph patch running on CPU1 with only a DAC task on CPU0.
#![allow(
    missing_docs,
    reason = "RTICX generates public task traits and initializers."
)]
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    wavegraph_fm1_core::peer::download()
}

#[cfg(target_os = "none")]
mod synth {
    use wavegraph_dsp::{Event, bytecode::Patch, dsp::Software};
    use wavegraph_fm1_core::{OUTPUT_FRAMES, Render};
    use wavegraph_monosynth::{Message, Monosynth};
    use wavegraph_sample::I24;

    pub(super) struct Tone(pub(super) Patch<'static, 'static, Message, I24, 1, Monosynth<'static>>);
    impl Render for Tone {
        type Block = [i32; OUTPUT_FRAMES];

        fn render(&mut self, first_frame: u64, out: &mut Self::Block) {
            let note = [Event {
                frame: 0,
                port: 0,
                message: Message::NoteOn {
                    note: 69,
                    velocity: 24,
                },
            }];
            let events = if first_frame == 0 { &note[..] } else { &[] };
            let mut buses = [[I24::default(); OUTPUT_FRAMES]; 1];
            self.0
                .render(&mut Software, &mut buses, &[events], |_, event| Err(event))
                .expect("one oscillator");
            for (out, sample) in out.iter_mut().zip(buses[0]) {
                *out = sample.inner();
            }
        }
    }
}

#[cfg(target_os = "none")]
#[fm1_rticx::app(dispatchers = [SOFT0])]
mod app {
    use super::synth::Tone;
    use fm1_audio::{FRAME_RATE, MasterLevel, Output};
    use fm1_rticx::bsp::Board;
    use static_cell::{ConstStaticCell, StaticCell};
    use wavegraph_dsp::bytecode::Program;
    use wavegraph_fm1_core::{Driver, OUTPUT_FRAMES, control::EditQueue, peer, start_output};
    use wavegraph_monosynth::{Monosynth, Tuning};

    wavegraph_fm1_core::dsp_worker!(static DSP: Tone);

    #[shared]
    struct Shared {}

    #[init]
    fn init() -> (Shared, TaskInits) {
        // SAFETY: RTICX init runs once on CPU0 before Board::take, with IRQs masked.
        assert_eq!(unsafe { peer::start() }, 1);
        let board = Board::take().expect("CPU0 board");
        static TUNING: Tuning = Tuning::new(FRAME_RATE.0, FRAME_RATE.1).expect("FM-1 clock");
        static VOICE: ConstStaticCell<Monosynth<'static>> =
            ConstStaticCell::new(Monosynth::new(&TUNING));
        static NODES: StaticCell<[&mut Monosynth<'static>; 1]> = StaticCell::new();
        static PROGRAM: StaticCell<Program<1, 1>> = StaticCell::new();
        // Version 1, call object 0 with zero signal inputs and one output.
        let program =
            PROGRAM.init_with(|| Program::decode(&[1, 1, 0, 0, 1]).expect("tone bytecode"));
        let patch = program
            .bind_nodes(NODES.init([VOICE.take()]))
            .expect("tone ports");

        static PCM: ConstStaticCell<[i32; OUTPUT_FRAMES]> =
            ConstStaticCell::new([0; OUTPUT_FRAMES]);
        static EDITS: ConstStaticCell<EditQueue<()>> = ConstStaticCell::new(EditQueue::new());
        let (_, host) = EDITS.take().split();
        let mut driver = DSP.connect(Tone(patch), PCM.take(), host);
        driver.request(|_, host| host.begin_block(|()| Ok(())));
        driver.wait_first_blocking().expect("CPU1 ready");
        let output = start_output(board.audio).expect("audio clock");
        (
            Shared {},
            TaskInits {
                audio: Audio { driver, output },
            },
        )
    }

    #[task(binds = ALNK0, priority = 2)]
    struct Audio {
        driver: Driver<[i32; OUTPUT_FRAMES], ()>,
        output: Output,
    }

    impl RticTask for Audio {
        fn exec(&mut self) {
            if self
                .driver
                .refill(&mut self.output, MasterLevel::MAX)
                .expect("CPU1 healthy")
                .is_some()
            {
                self.driver.request(|_, host| host.begin_block(|()| Ok(())));
            }
        }
    }
}
