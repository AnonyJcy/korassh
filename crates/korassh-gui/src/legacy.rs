//! The carry-over from an OmnySSH install, run once before anything else starts.
//!
//! Tauri keys an app's own data by its bundle identifier, and KoraSSH ships under a
//! new one, so its directories start empty on first launch. The engine brings
//! `hosts.toml`, `snippets.toml` and `config.toml` across the first time it resolves
//! the config directory (`platform::app_config_dir`); this module brings the desktop's
//! preference store — the file the store plugin reads the moment the page hydrates its
//! settings.
//!
//! Copy only: the old location is never written to or removed.

use std::path::{Path, PathBuf};

/// The identifier OmnySSH shipped under.
pub const LEGACY_IDENTIFIER: &str = "com.timhartmann.omnyssh";
/// The identifier KoraSSH ships under — `identifier` in tauri.conf.json.
pub const IDENTIFIER: &str = "com.anonyjcy.korassh";
/// The frontend's preference store, as the store plugin names it (`load("settings.json")`).
const PREFERENCES_FILE: &str = "settings.json";

/// Copies OmnySSH's preference store into KoraSSH's data directory when KoraSSH has
/// none yet. `base` is the platform's per-user data directory (`dirs::data_dir()`),
/// under which Tauri keeps one folder per identifier. Returns the file it wrote.
pub fn migrate_preferences(base: &Path) -> Option<PathBuf> {
    let from = base.join(LEGACY_IDENTIFIER).join(PREFERENCES_FILE);
    let to_dir = base.join(IDENTIFIER);
    let to = to_dir.join(PREFERENCES_FILE);
    if !from.is_file() || to.exists() {
        return None;
    }
    std::fs::create_dir_all(&to_dir).ok()?;
    std::fs::copy(&from, &to).ok().map(|_| to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brings_the_preferences_over_once_and_leaves_the_source_alone() {
        let base = tempfile::tempdir().expect("tempdir");
        let from = base.path().join(LEGACY_IDENTIFIER).join(PREFERENCES_FILE);
        std::fs::create_dir_all(from.parent().unwrap()).expect("mkdir");
        std::fs::write(&from, br#"{"theme":"light"}"#).expect("write");

        let written = migrate_preferences(base.path()).expect("copied");
        assert_eq!(written, base.path().join(IDENTIFIER).join(PREFERENCES_FILE));
        assert_eq!(std::fs::read(&written).unwrap(), br#"{"theme":"light"}"#);
        // The source is still there, and a second launch changes nothing.
        assert!(from.is_file());
        assert!(migrate_preferences(base.path()).is_none());
    }

    #[test]
    fn never_overwrites_preferences_korassh_already_has() {
        let base = tempfile::tempdir().expect("tempdir");
        let legacy = base.path().join(LEGACY_IDENTIFIER).join(PREFERENCES_FILE);
        std::fs::create_dir_all(legacy.parent().unwrap()).expect("mkdir");
        std::fs::write(&legacy, br#"{"theme":"light"}"#).expect("write");
        let own = base.path().join(IDENTIFIER).join(PREFERENCES_FILE);
        std::fs::create_dir_all(own.parent().unwrap()).expect("mkdir");
        std::fs::write(&own, br#"{"theme":"dark"}"#).expect("write");

        assert!(migrate_preferences(base.path()).is_none());
        assert_eq!(std::fs::read(&own).unwrap(), br#"{"theme":"dark"}"#);
    }

    #[test]
    fn a_first_install_without_omnyssh_is_left_alone() {
        let base = tempfile::tempdir().expect("tempdir");
        assert!(migrate_preferences(base.path()).is_none());
        assert!(!base.path().join(IDENTIFIER).exists());
    }
}
