//! WaveGraph monosynth, house organ, 303 and 909 firmware for the FM-1.
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(not(target_os = "none"))]
fn main() {
    eprintln!("wavegraph-fm1 runs on the FM-1 target");
}

#[cfg(any(target_os = "none", test))]
mod bass;
#[cfg(any(target_os = "none", test))]
mod bumper;
#[cfg(any(target_os = "none", test))]
mod capture;
#[cfg(any(target_os = "none", test))]
mod envelope;
#[cfg(target_os = "none")]
mod firmware;
#[cfg(any(target_os = "none", test))]
mod fx;
#[cfg(any(target_os = "none", test))]
mod inserts;
#[cfg(any(target_os = "none", test))]
mod instrument;
#[cfg(any(target_os = "none", test))]
mod keys;
#[cfg(any(target_os = "none", test))]
mod mixer;
#[cfg(any(target_os = "none", test))]
mod organ;
#[cfg(any(target_os = "none", test))]
mod overload;
#[cfg(any(target_os = "none", test))]
mod panel;
#[cfg(target_os = "none")]
mod screen;
#[cfg(any(target_os = "none", test))]
mod sequencer;

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    fm1_rticx::bsp::enter_download_mode()
}
