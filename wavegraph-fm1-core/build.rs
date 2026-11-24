//! Supply the dual-core reset prefix, stack switch, and linker script to the final binary.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo::rerun-if-env-changed=PI32V2_BIN");
    println!("cargo::rerun-if-changed=runtime/start.S");
    println!("cargo::rerun-if-changed=wavegraph-fm1.ld");
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("pi32v2") {
        return;
    }
    let bin = PathBuf::from(env::var_os("PI32V2_BIN").expect("pi32v2 LLVM"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build directory"));
    let object = out.join("start.o");
    assert!(
        Command::new(bin.join("clang"))
            .args([
                "--target=pi32v2",
                "-mcpu=ac791n",
                "-c",
                "runtime/start.S",
                "-o"
            ])
            .arg(&object)
            .status()
            .expect("pi32v2 clang")
            .success()
    );
    assert!(
        Command::new(bin.join("llvm-ar"))
            .arg("crs")
            .arg(out.join("libwavegraph_start.a"))
            .arg(object)
            .status()
            .expect("llvm-ar")
            .success()
    );
    println!("cargo::rustc-link-search=native={}", out.display());
    println!("cargo::rustc-link-lib=static=wavegraph_start");
    println!(
        "cargo::rustc-link-search=native={}",
        env!("CARGO_MANIFEST_DIR")
    );
}
