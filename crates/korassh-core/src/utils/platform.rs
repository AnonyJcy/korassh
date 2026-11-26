use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Rolling log files older than this many days are pruned at startup.
pub const LOG_RETENTION_DAYS: u64 = 7;

/// Prefix shared by every rolling log file (`korassh.log`, `korassh.log.YYYY-MM-DD`).
const LOG_FILE_PREFIX: &str = "korassh.log";

static MIGRATED_LEGACY: OnceLock<()> = OnceLock::new();

/// Copies configuration files from a legacy `omnyssh` directory into `target`
/// when `target` does not yet hold `hosts.toml` or `snippets.toml`.
///
/// Non-destructive: the legacy directory is never modified or removed.
pub fn migrate_legacy_dir(legacy: &Path, target: &Path) -> bool {
    if !legacy.is_dir() {
        return false;
    }
    if target.join("hosts.toml").exists() || target.join("snippets.toml").exists() {
        return false;
    }
    if std::fs::create_dir_all(target).is_err() {
        return false;
    }
    let mut copied_any = false;
    for name in ["hosts.toml", "snippets.toml", "config.toml"] {
        let src = legacy.join(name);
        let dst = target.join(name);
        if src.is_file() && !dst.exists() && std::fs::copy(&src, &dst).is_ok() {
            copied_any = true;
        }
    }
    copied_any
}

/// Returns the path to the user's SSH config file.
///
/// - Linux / macOS: `~/.ssh/config`
/// - Windows:       `%USERPROFILE%\.ssh\config`
///
/// Uses the `dirs` crate so we never hardcode `~`.
pub fn ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("config"))
}

/// Returns the application config directory, migrating legacy `omnyssh` configs
/// once on first launch if present.
///
/// - Linux:   `~/.config/korassh/`
/// - macOS:   `~/Library/Application Support/korassh/`
/// - Windows: `%APPDATA%\korassh\`
pub fn app_config_dir() -> Option<PathBuf> {
    let base = dirs::config_dir()?;
    let target = base.join("korassh");
    MIGRATED_LEGACY.get_or_init(|| {
        let legacy = base.join("omnyssh");
        if migrate_legacy_dir(&legacy, &target) {
            tracing::info!(
                from = %legacy.display(),
                to = %target.display(),
                "migrated legacy omnyssh configuration to korassh"
            );
        }
    });
    Some(target)
}

/// Returns the path to the main application config file.
pub fn app_config_path() -> Option<PathBuf> {
    app_config_dir().map(|d| d.join("config.toml"))
}

/// Returns the path to the hosts config file.
pub fn hosts_config_path() -> Option<PathBuf> {
    app_config_dir().map(|d| d.join("hosts.toml"))
}

/// Returns the path to the snippets config file.
pub fn snippets_config_path() -> Option<PathBuf> {
    app_config_dir().map(|d| d.join("snippets.toml"))
}

/// Removes rolling log files in `log_dir` older than `max_age_days`.
pub fn cleanup_old_logs(log_dir: &Path, max_age_days: u64) -> usize {
    let entries = match std::fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };

    let max_age = Duration::from_secs(max_age_days * 24 * 60 * 60);
    let now = SystemTime::now();
    let mut removed = 0;

    for entry in entries.flatten() {
        let path = entry.path();

        let is_log_file = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with(LOG_FILE_PREFIX))
            .unwrap_or(false);
        if !is_log_file {
            continue;
        }

        let modified = match entry.metadata().and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };

        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age > max_age && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrates_legacy_omnyssh_configs_once_without_modifying_source() {
        let root = TempDir::new().expect("tempdir");
        let legacy = root.path().join("omnyssh");
        let target = root.path().join("korassh");
        std::fs::create_dir_all(&legacy).expect("mkdir");
        std::fs::write(legacy.join("hosts.toml"), "hosts = []\n").expect("write hosts");
        std::fs::write(legacy.join("snippets.toml"), "snippets = []\n").expect("write snippets");

        assert!(migrate_legacy_dir(&legacy, &target));
        assert!(target.join("hosts.toml").exists());
        assert!(target.join("snippets.toml").exists());
        // Source remains intact and second call is a no-op.
        assert!(legacy.join("hosts.toml").exists());
        assert!(!migrate_legacy_dir(&legacy, &target));
    }
}
