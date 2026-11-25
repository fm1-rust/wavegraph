//! Hardware assumptions for enabling USB MIDI on the FM-1.

use crate::usb_fault::{BYTES, MAGIC, encode, message};
use core::{fmt::Write as _, panic::PanicInfo};
use fm1::{
    Board,
    display::SpiClock,
    time::{Timers, cycles},
    usb_midi::{Identity, MidiDevice, UsbPort},
};
use fm1_graphics::{TextGrid, palette};

const WRITING: u32 = u32::from_le_bytes(*b"WUP1");
const COMMITTED: u32 = u32::from_le_bytes(MAGIC);

unsafe extern "C" {
    // These two leaf routines mask their offsets to the 16 aligned words of
    // the linker-reserved slot. No Rust reference addresses that storage.
    fn wavegraph_fault_read(offset: u32) -> u32;
    fn wavegraph_fault_write(offset: u32, value: u32);
    // Returns the hardware core number without touching memory.
    safe fn fm1_core_id() -> u32;
    fn fm1_cli();
    fn fm1_cpu1_probe_stop();
    fn fm1_ssync();
}

fn measured_revision() -> bool {
    // SAFETY: this module is compiled only in an FM-1 target image. WL82.h
    // defines JL_INTEST->CHIP_ID as this aligned, readable register outside
    // Rust allocations. Reading it has no side effects on RAM.
    unsafe { core::ptr::without_provenance::<u32>(0x0001_0200).read_volatile() == 0x0000_6f01 }
}

/// Save the first terminal copy fault using only CPU0 and reserved SRAM.
/// The BSP’s explicit fault path quarantines the engine and withdraws D+ first.
pub(super) fn save_copy_fault(info: &PanicInfo<'_>) {
    // Use the pinned BSP's source location. Formatting arbitrary panic
    // arguments could call application Display implementations during recovery.
    let from_copy_driver = info.location().is_some_and(|site| {
        let file = site.file();
        file.ends_with("/usb_midi/copied.rs") || file.ends_with("\\usb_midi\\copied.rs")
    });
    if fm1_core_id() != 0 || !from_copy_driver {
        return;
    }
    // SAFETY: the caller is the terminal CPU0 panic path, the BSP has
    // poisoned its sole copy engine and detached USB. Mask CPU0 interrupts
    // and hold CPU1 before waiting. Neither operation submits a DMA task.
    unsafe {
        fm1_cli();
        fm1_cpu1_probe_stop();
    }
    let status = copy_status();
    persist_copy_fault(status);
    // No loader or CRT may reuse this RAM while the last task is outstanding.
    // On this exclusive engine DONE is cleared before each submission. A
    // late completion can release the wait. A stuck or errored engine stays
    // parked, with the record and every possible DMA target still reserved.
    while copy_status() & 0xa0 != 0x80 {
        core::hint::spin_loop();
    }
    // SAFETY: successful hardware completion orders access to the active
    // task under the board contract. Finish the acquire before permitting
    // the ordinary panic handler to request a warm download-mode reset.
    unsafe { fm1_ssync() };
}

fn copy_status() -> u32 {
    // SAFETY: this aligned JL_DMA->CON register is outside Rust allocations.
    // CPU0 only reads it here. Reading it does not clear the completion flag.
    unsafe { core::ptr::without_provenance::<u32>(0x0004_1300).read_volatile() }
}

fn persist_copy_fault(status: u32) {
    let mut text = heapless::String::<55>::new();
    if write!(text, "USB0 COPY FAULT CON {status:08X}").is_err() {
        return;
    }
    let Some(bytes) = encode(&text) else { return };
    // SAFETY: the sole CPU0 copy driver is terminating. Its descriptor and
    // every endpoint address are disjoint from the fixed crash slot. No
    // handler, DMA transfer, or CPU1 task owns that slot. This argument
    // assumes the board-level bus confinement contract still holds. Each assembly
    // store is bounded and ordered. A reset midway preserves an incomplete
    // marker. Only a fully written payload and CRC receive the commit word.
    unsafe {
        wavegraph_fault_write(0, WRITING);
        for (offset, &word) in (4u32..64).step_by(4).zip(&bytes.as_chunks::<4>().0[1..]) {
            wavegraph_fault_write(offset, u32::from_le_bytes(word));
        }
        wavegraph_fault_write(0, COMMITTED);
    }
}

/// A powered restart displays a recorded fault before USB can attach again.
/// Returning the board continues normal initialization on a clean boot.
pub(super) fn recover(mut board: Board, id: Identity) -> Board {
    if !measured_revision() {
        return board;
    }
    // SAFETY: Board establishes the FM-1 map and unique CPU0 startup. The
    // slot is outside initialized Rust allocations, both stacks, and the
    // bootloader mailbox. Read cold SRAM as arbitrary machine words through
    // the assembly shim. A marker, length, ASCII and checksum are validated
    // before interpreting any of it. A record can grant no memory access.
    let marker = unsafe { wavegraph_fault_read(0) };
    if marker != COMMITTED && marker != WRITING {
        return board;
    }
    let mut bytes = [0; BYTES];
    for (offset, word) in (0u32..64).step_by(4).zip(bytes.as_chunks_mut::<4>().0) {
        // SAFETY: same reserved slot and CPU0 ownership as the marker read.
        *word = unsafe { wavegraph_fault_read(offset) }.to_le_bytes();
    }
    // The checked path on fm1 0.2.0 explicitly detaches a possibly active
    // loader connection. Keep all copy/endpoint storage untouched this boot.
    let _ = board
        .take_usb()
        .expect("one recovery port")
        .try_into_midi(id);
    let Ok(mut lcd) = board.display.init_with_clock(SpiClock::Div1) else {
        wavegraph_fm1_core::peer::download()
    };
    let mut grid = TextGrid::new();
    grid.text(0, 0, "WAVEGRAPH USB FAULT", palette::RED);
    if let Some(reason) = message(&bytes) {
        for (row, chunk) in reason.as_bytes().chunks(20).enumerate() {
            grid.text(0, row + 3, chunk, palette::WHITE);
        }
    } else {
        grid.text(0, 3, "DAMAGED FAULT RECORD", palette::RED);
    }
    grid.text(0, 10, "PHOTO THEN HOLD PLAY", palette::CYAN);
    grid.text(0, 11, "2S TO CLEAR AND EXIT", palette::CYAN);
    if lcd.fill_rect(0, 0, 240, 240, 0).is_err() || grid.paint(&mut lcd).is_err() {
        wavegraph_fm1_core::peer::download()
    }
    let mut since = None;
    loop {
        if board.panel.button().is_pressed() {
            let now = cycles();
            if now.wrapping_sub(*since.get_or_insert(now)) >= 2 * Timers::CRYSTAL_HZ {
                // SAFETY: the operator acknowledged this CPU0-owned record.
                // The checked recovery path has submitted no copy jobs.
                unsafe { wavegraph_fault_write(0, 0) };
                wavegraph_fm1_core::peer::download()
            }
        } else {
            since = None;
        }
    }
}

/// Board-specific entry point for music tasks. The only input that can move
/// into the controller is the unique non-Send CPU0 peripheral token.
pub(super) fn start(port: UsbPort, identity: Identity) -> Option<MidiDevice> {
    if !measured_revision() {
        return port.try_into_midi(identity).ok();
    }

    // SAFETY: WaveGraph runs under a trusted FM-1 loader. On the supported
    // 0x6f01 profile it dedicates the idle WL82 copy engine and IRQ to this
    // BSP for the entire boot. CPU1 runs only the DSP worker; ALNK and SPI
    // use independent engines. USB owns its permanent endpoint storage even
    // when two hardware agents see different bytes of one packet. No host
    // length or byte chooses an address or enlarges the 68/16-byte copy.
    // Silicon assumptions: USB0 stays inside its arenas even for rejected
    // packets, reset and reconnect. Fresh copy DONE plus SSYNC retires every
    // descriptor/staging access. The loader must quiesce both masters before
    // a warm boot reuses RAM. The chip ID selects this hardware profile.
    // The watermark, identity and intact guards do not prove those premises.
    // See the firmware README, USB MIDI errata, for witnesses and falsifiers.
    Some(unsafe { port.into_midi_with_dma_copy(identity) })
}
