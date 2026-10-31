//! Reads the display name from `brand.toml` at the repository root, so no source file has to spell it.
use std::{env, fs, path::PathBuf};

fn main() {
    let manifest =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let brand = manifest.join("../../brand.toml");
    println!("cargo:rerun-if-changed={}", brand.display());
    let text = fs::read_to_string(&brand)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", brand.display()));
    let name = text
        .lines()
        .filter_map(|l| l.split_once('='))
        .find(|(key, _)| key.trim() == "name")
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| panic!("{} needs a non-empty name = \"...\" line", brand.display()));
    println!("cargo:rustc-env=APP_DISPLAY_NAME={name}");
}
