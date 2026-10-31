//! The display name lives in `app.toml` and nowhere else.
use std::{
    fs,
    path::{Path, PathBuf},
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn app_name() -> String {
    let text = fs::read_to_string(repo_root().join("app.toml")).expect("app.toml");
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "name")
        .map(|(_, v)| v.trim().trim_matches('"').to_string())
        .expect("app.toml has a name")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if matches!(name.as_str(), "target" | "dist" | ".git" | "claude.session") {
            continue;
        }
        out.push(path.clone());
        if path.is_dir() {
            files(&path, out);
        }
    }
}

fn relative(path: &Path) -> String {
    let root = repo_root().canonicalize().unwrap();
    let full = path.canonicalize().unwrap();
    full.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Paths (and file contents) that mention `needle`, skipping `allowed` prefixes.
fn mentions(needle: &str, allowed: &[&str]) -> Vec<String> {
    let mut all = vec![];
    files(&repo_root(), &mut all);
    let mut hits = vec![];
    for path in all {
        let rel = relative(&path);
        if rel == "app.toml" || allowed.iter().any(|ok| rel.starts_with(ok)) {
            continue;
        }
        if rel.contains(needle) {
            hits.push(format!("{rel} (file name)"));
        }
        if path.is_file() {
            if let Ok(text) = fs::read_to_string(&path) {
                if text.contains(needle) {
                    hits.push(rel);
                }
            }
        }
    }
    hits
}

#[test]
fn the_display_name_appears_only_in_the_app_file() {
    let hits = mentions(&app_name(), &[]);
    assert!(
        hits.is_empty(),
        "the name belongs in app.toml only: {hits:?}"
    );
}

#[test]
fn the_cargo_version_matches_the_version_in_the_app_file() {
    let cargo = fs::read_to_string(repo_root().join("Cargo.toml")).unwrap();
    let in_cargo = cargo
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|v| v.strip_suffix('"'))
        .expect("Cargo.toml has a version");
    assert_eq!(
        in_cargo,
        guide_core::APP_VERSION,
        "bump app.toml and Cargo.toml together"
    );
}
