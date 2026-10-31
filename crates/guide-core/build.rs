//! Reads the display name and the project extension from `app.toml` at the repository root, so
//! no source file has to spell them.
use std::{env, fs, path::PathBuf};

fn main() {
    let manifest =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let file = manifest.join("../../app.toml");
    println!("cargo:rerun-if-changed={}", file.display());
    let text =
        fs::read_to_string(&file).unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
    let value = |key: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| panic!("{} needs a non-empty {key} = \"...\" line", file.display()))
    };
    println!("cargo:rustc-env=APP_DISPLAY_NAME={}", value("name"));
    let version = value("version");
    let numbers = version.split(['-', '+']).next().unwrap_or_default();
    assert!(
        numbers.split('.').count() == 3 && numbers.split('.').all(|n| n.parse::<u32>().is_ok()),
        "version must look like 1.2.3 (optionally followed by -suffix)"
    );
    println!("cargo:rustc-env=APP_VERSION={version}");
    for (key, env_name) in [
        ("project_extension", "PROJECT_EXTENSION"),
        ("edit_extension", "EDIT_EXTENSION"),
    ] {
        let ext = value(key);
        assert!(
            ext.chars().all(|c| c.is_ascii_alphanumeric()),
            "{key} must be letters and digits only, without a dot"
        );
        println!("cargo:rustc-env={env_name}={ext}");
    }
}
