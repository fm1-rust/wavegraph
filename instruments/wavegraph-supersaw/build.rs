//! Enable the native mixer only for an explicitly selected pi32v2 build.

fn main() {
    println!("cargo:rustc-check-cfg=cfg(pi32v2_simd)");
    if std::env::var_os("CARGO_FEATURE_PI32V2_SIMD").is_some()
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("pi32v2")
    {
        println!("cargo:rustc-cfg=pi32v2_simd");
    }
}
