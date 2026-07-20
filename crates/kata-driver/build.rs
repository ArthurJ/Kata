//! Build script do `kata-driver`.
//!
//! Define `KATA_BUILD_ROOT` com o path absoluto do workspace root
//! (diretório que contém `Cargo.toml` do workspace). Usado pelo driver
//! para descobrir `target/<profile>/libkata_rt.a` para linkagem AOT.

use std::path::PathBuf;

fn main() {
    // CARGO_MANIFEST_DIR é o diretório de kata-driver:
    // <workspace>/crates/kata-driver
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR não definido");
    let manifest_dir = PathBuf::from(manifest_dir);
    // workspace root = manifest_dir/../../
    let workspace_root = manifest_dir
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .expect("não foi possível determinar workspace root");

    println!(
        "cargo:rustc-env=KATA_BUILD_ROOT={}",
        workspace_root.display()
    );
}
