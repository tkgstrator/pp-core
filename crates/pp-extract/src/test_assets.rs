//! Local, copyright-protected parity fixtures used only by tests.

use std::path::{Path, PathBuf};

fn required() -> bool {
    match std::env::var("PPEXTRACTOR_REQUIRE_ROM_PARITY").as_deref() {
        Ok("1") => true,
        Ok("0") | Err(std::env::VarError::NotPresent) => false,
        _ => panic!("PPEXTRACTOR_REQUIRE_ROM_PARITY must be 0 or 1"),
    }
}

pub fn root() -> Option<PathBuf> {
    match std::env::var_os("PPEXTRACTOR_ASSETS_ROOT") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ if required() => {
            panic!("PPEXTRACTOR_REQUIRE_ROM_PARITY=1 requires PPEXTRACTOR_ASSETS_ROOT")
        }
        _ => {
            eprintln!("skipping local ROM parity: set PPEXTRACTOR_ASSETS_ROOT to enable it");
            None
        }
    }
}

pub fn available(path: &Path) -> bool {
    if path.exists() {
        return true;
    }
    if required() {
        panic!("required ROM parity asset missing: {}", path.display());
    }
    eprintln!("skipping unavailable ROM parity asset: {}", path.display());
    false
}

pub fn rom(name: &str) -> Option<PathBuf> {
    let base = root()?.join("roms");
    let revision_file = match name {
        "pp4" => Some("pp4_v1.gba"),
        "pp6" => Some("pp6_v0.gba"),
        _ => None,
    };
    if let Some(path) = revision_file.map(|file| base.join(file))
        && path.is_file()
    {
        return Some(path);
    }
    let path = base.join(format!("{name}.gba"));
    available(&path).then_some(path)
}

pub fn oracle(name: &str, category: &str, marker: &str) -> Option<PathBuf> {
    let base = root()?.join("extracted");
    let legacy = base.join(name).join(category);
    if legacy.join(marker).exists() {
        return Some(legacy);
    }
    if let Some(number) = name.strip_prefix("pp") {
        let relative = match category {
            "portraits" => "images/portraits",
            "scripts" => "scripts/native",
            _ => category,
        };
        let path = base.join(number).join(relative);
        if path.join(marker).exists() {
            return Some(path);
        }
    }
    available(&legacy.join(marker)).then_some(legacy)
}
