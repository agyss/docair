//! Builds this crate to WebAssembly for the page whenever it is built for the host (docair-702.42):
//! the module is never kept in the repository, so it is always built from the code as it is. Built
//! for wasm32 (the build this starts), it does nothing.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("wasm32") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let workspace = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/docair-layout-wasm");
    for path in [
        "src",
        "Cargo.toml",
        "../docair-layout/src",
        "../docair-layout/Cargo.toml",
    ] {
        println!("cargo:rerun-if-changed={}", manifest.join(path).display());
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    let target_dir = out.join("wasm-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    // Its own target folder, so it does not wait on the build that runs this; the flags of the
    // host build are not for wasm32.
    let status = Command::new(cargo)
        .current_dir(workspace)
        .args([
            "build",
            "-p",
            "docair-layout-wasm",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            "wasm",
            "--offline",
            "--target-dir",
        ])
        .arg(&target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .status()
        .expect("cargo runs");
    assert!(
        status.success(),
        "building layout.wasm failed: the wasm32 target is needed (rustup target add wasm32-unknown-unknown)"
    );
    std::fs::copy(
        target_dir.join("wasm32-unknown-unknown/wasm/docair_layout_wasm.wasm"),
        out.join("layout.wasm"),
    )
    .expect("the built module");
}
