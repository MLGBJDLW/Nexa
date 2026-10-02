#[allow(dead_code)]
#[path = "src/provenance.rs"]
mod provenance;

fn main() {
    println!(
        "cargo:rustc-env=NEXA_EVAL_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap()
    );
    println!(
        "cargo:rustc-env=NEXA_EVAL_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    let compiler = std::process::Command::new(std::env::var_os("RUSTC").unwrap())
        .arg("--version")
        .output()
        .expect("Rust compiler version");
    assert!(
        compiler.status.success(),
        "Rust compiler version must be available"
    );
    println!(
        "cargo:rustc-env=NEXA_EVAL_RUSTC_VERSION={}",
        String::from_utf8_lossy(&compiler.stdout).trim()
    );
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = provenance::repository_root(&manifest)
        .expect("agent evaluation builds require an inspectable Git checkout");
    let source = provenance::capture_checkout(&root)
        .expect("agent evaluation source fingerprint must be complete");
    for path in provenance::cargo_watch_paths(&root, &source).expect("source watch paths") {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rustc-env=NEXA_EVAL_BUILD_SHA={}", source.source_sha);
    println!(
        "cargo:rustc-env=NEXA_EVAL_BUILD_DIRTY={}",
        source.source_dirty
    );
    println!(
        "cargo:rustc-env=NEXA_EVAL_BUILD_FINGERPRINT={}",
        source.source_fingerprint
    );
}
