//! End-to-end encrypted backup import/export and WebDAV / Git synchronization.
//!
//! Exports manual hosts (with decrypted passwords inside the encrypted payload so
//! backups are portable across machines), command snippets, and general settings
//! into an Argon2id + AES-256-GCM encrypted JSON envelope (`.korabak`).
//!
//! Supports:
//! - Direct file / string export & import (Merge or Replace mode)
//! - WebDAV sync (`PUT` / `GET` over HTTPS with Basic Auth; password stored encrypted via vault)
//! - Git repository sync (writes encrypted `.korabak` file, commits, pushes/pulls via `git`)

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::config::app_config::AppConfig;
use crate::config::snippets::{load_snippets, save_snippets, Snippet};
use crate::config::vault::{self, EncryptedEnvelope, VaultDir};
use crate::config::{load_hosts, save_hosts};
use crate::ssh::client::{Host, HostSource};
use crate::utils::platform;

const SYNC_CONFIG_FILE: &str = "sync.toml";
const DEFAULT_SYNC_FILENAME: &str = "korassh-sync.korabak";
const WEBDAV_TIMEOUT: Duration = Duration::from_secs(30);

/// Synchronization backend chosen by the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncProvider {
    #[default]
    None,
    WebDav,
    Git,
}

/// Persistent synchronization settings stored in `<config_dir>/sync.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    #[serde(default)]
    pub provider: SyncProvider,
    #[serde(default)]
    pub webdav_url: String,
    #[serde(default)]
    pub webdav_username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webdav_password: Option<String>,
    #[serde(default = "default_sync_filename")]
    pub webdav_remote_path: String,
    #[serde(default)]
    pub git_repo_dir: String,
    #[serde(default = "default_git_remote")]
    pub git_remote: String,
    #[serde(default = "default_git_branch")]
    pub git_branch: String,
    #[serde(default = "default_sync_filename")]
    pub git_file_name: String,
}

fn default_sync_filename() -> String {
    DEFAULT_SYNC_FILENAME.to_string()
}

fn default_git_remote() -> String {
    "origin".to_string()
}

fn default_git_branch() -> String {
    "main".to_string()
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            provider: SyncProvider::None,
            webdav_url: String::new(),
            webdav_username: String::new(),
            webdav_password: None,
            webdav_remote_path: default_sync_filename(),
            git_repo_dir: String::new(),
            git_remote: default_git_remote(),
            git_branch: default_git_branch(),
            git_file_name: default_sync_filename(),
        }
    }
}

/// Inner JSON payload encrypted inside an [`EncryptedEnvelope`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupPayload {
    pub version: u8,
    pub exported_at: String,
    pub hosts: Vec<Host>,
    pub snippets: Vec<Snippet>,
    #[serde(default)]
    pub app_config: AppConfig,
}

/// Summary returned after importing or pulling an encrypted bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub hosts_imported: usize,
    pub snippets_imported: usize,
}

fn sync_config_path() -> Result<PathBuf> {
    let dir = platform::app_config_dir().context("cannot determine app config directory")?;
    Ok(dir.join(SYNC_CONFIG_FILE))
}

/// Loads `sync.toml` and decrypts `webdav_password` when the local vault is unlocked.
pub fn load_sync_config() -> Result<SyncConfig> {
    let path = sync_config_path()?;
    if !path.exists() {
        return Ok(SyncConfig::default());
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut cfg: SyncConfig =
        toml::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    if let (Ok(v), Some(ref raw_pw)) = (VaultDir::default_dir(), &cfg.webdav_password) {
        if !raw_pw.is_empty() && vault::is_encrypted(raw_pw) {
            if let Ok(Some(plain)) = v.try_decrypt_secret(raw_pw) {
                cfg.webdav_password = Some(plain);
            }
        }
    }
    Ok(cfg)
}

/// Persists `sync.toml`, encrypting `webdav_password` with the local vault.
pub fn save_sync_config(config: &SyncConfig) -> Result<()> {
    let path = sync_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut to_write = config.clone();
    if let Some(ref pw) = to_write.webdav_password {
        if pw.is_empty() {
            to_write.webdav_password = None;
        } else if !vault::is_encrypted(pw) {
            let v = VaultDir::default_dir()?;
            to_write.webdav_password = Some(v.encrypt_secret(pw)?);
        }
    }
    let content = toml::to_string_pretty(&to_write).context("serialize sync.toml")?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("rename to {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Packs current manual hosts, snippets, and app config into an E2E encrypted JSON bundle.
pub fn export_encrypted_backup(passphrase: &str) -> Result<String> {
    if !vault::status().unlocked {
        bail!("unlock the local vault before exporting so saved passwords can be included");
    }
    let hosts = load_hosts()?;
    let snippets = load_snippets()?;
    let app_config = crate::config::app_config::load_app_config(None).unwrap_or_default();
    encrypt_backup_payload(
        &BackupPayload {
            version: 1,
            exported_at: Utc::now().to_rfc3339(),
            hosts,
            snippets,
            app_config,
        },
        passphrase,
    )
}

/// Serializes and encrypts a [`BackupPayload`] with `passphrase`.
pub fn encrypt_backup_payload(payload: &BackupPayload, passphrase: &str) -> Result<String> {
    let json = serde_json::to_vec(payload).context("serialize backup payload")?;
    let envelope = vault::encrypt_with_passphrase(&json, passphrase)?;
    serde_json::to_string_pretty(&envelope).context("serialize encrypted envelope")
}

/// Decrypts an encrypted JSON bundle back into a [`BackupPayload`].
pub fn decrypt_backup_payload(bundle_json: &str, passphrase: &str) -> Result<BackupPayload> {
    let envelope: EncryptedEnvelope =
        serde_json::from_str(bundle_json).context("invalid encrypted backup format")?;
    let plain = vault::decrypt_with_passphrase(&envelope, passphrase)?;
    let payload: BackupPayload =
        serde_json::from_slice(&plain).context("corrupted backup payload JSON")?;
    if payload.version != 1 {
        bail!("unsupported backup payload version {}", payload.version);
    }
    Ok(payload)
}

/// Merges or replaces `existing` hosts with `incoming` manual hosts by name.
pub fn merge_backup_hosts(
    mut existing: Vec<Host>,
    incoming: Vec<Host>,
    replace: bool,
) -> Vec<Host> {
    let incoming_manual: Vec<Host> = incoming
        .into_iter()
        .map(|mut h| {
            h.source = HostSource::Manual;
            h
        })
        .collect();
    if replace {
        return incoming_manual;
    }
    for host in incoming_manual {
        if let Some(slot) = existing.iter_mut().find(|h| h.name == host.name) {
            *slot = host;
        } else {
            existing.push(host);
        }
    }
    existing
}

/// Merges or replaces `existing` snippets with `incoming` snippets by name.
pub fn merge_backup_snippets(
    mut existing: Vec<Snippet>,
    incoming: Vec<Snippet>,
    replace: bool,
) -> Vec<Snippet> {
    if replace {
        return incoming;
    }
    for snippet in incoming {
        if let Some(slot) = existing.iter_mut().find(|s| s.name == snippet.name) {
            *slot = snippet;
        } else {
            existing.push(snippet);
        }
    }
    existing
}

/// Decrypts an encrypted bundle and persists hosts + snippets into local storage.
pub fn import_encrypted_backup(
    bundle_json: &str,
    passphrase: &str,
    replace: bool,
) -> Result<ImportSummary> {
    let payload = decrypt_backup_payload(bundle_json, passphrase)?;
    let hosts_imported = payload.hosts.len();
    let snippets_imported = payload.snippets.len();

    let existing_hosts = load_hosts().unwrap_or_default();
    let merged_hosts = merge_backup_hosts(existing_hosts, payload.hosts, replace);
    save_hosts(&merged_hosts)?;

    let existing_snippets = load_snippets().unwrap_or_default();
    let merged_snippets = merge_backup_snippets(existing_snippets, payload.snippets, replace);
    save_snippets(&merged_snippets)?;

    Ok(ImportSummary {
        hosts_imported,
        snippets_imported,
    })
}

fn build_webdav_target_url(base_url: &str, remote_path: &str) -> Result<String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() || (!base.starts_with("http://") && !base.starts_with("https://")) {
        bail!("WebDAV URL must start with http:// or https://");
    }
    let rel = remote_path.trim().trim_start_matches('/');
    let rel = if rel.is_empty() {
        DEFAULT_SYNC_FILENAME
    } else {
        rel
    };
    Ok(format!("{base}/{rel}"))
}

fn webdav_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(WEBDAV_TIMEOUT)
        .build()
        .context("build HTTP client for WebDAV")
}

/// Uploads an E2E encrypted snapshot to the configured WebDAV endpoint.
pub async fn webdav_push_payload(config: &SyncConfig, bundle_json: &str) -> Result<String> {
    let url = build_webdav_target_url(&config.webdav_url, &config.webdav_remote_path)?;
    let client = webdav_client()?;
    let mut req = client
        .put(&url)
        .header("Content-Type", "application/json; charset=utf-8")
        .body(bundle_json.to_string());
    if !config.webdav_username.is_empty() {
        let pass = config
            .webdav_password
            .as_deref()
            .filter(|p| !vault::is_encrypted(p));
        req = req.basic_auth(&config.webdav_username, pass);
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("WebDAV PUT {url}"))?;
    if !resp.status().is_success() {
        bail!("WebDAV upload failed with HTTP {}", resp.status());
    }
    Ok(format!("Uploaded encrypted snapshot to {url}"))
}

/// Downloads an E2E encrypted snapshot from the configured WebDAV endpoint.
pub async fn webdav_pull_payload(config: &SyncConfig) -> Result<String> {
    let url = build_webdav_target_url(&config.webdav_url, &config.webdav_remote_path)?;
    let client = webdav_client()?;
    let mut req = client.get(&url);
    if !config.webdav_username.is_empty() {
        let pass = config
            .webdav_password
            .as_deref()
            .filter(|p| !vault::is_encrypted(p));
        req = req.basic_auth(&config.webdav_username, pass);
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("WebDAV GET {url}"))?;
    if !resp.status().is_success() {
        bail!("WebDAV download failed with HTTP {}", resp.status());
    }
    resp.text().await.context("read WebDAV response body")
}

fn run_git(repo_dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_dir)
        .args(args)
        .output()
        .with_context(|| format!("execute git {:?}", args))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        bail!("git {} failed: {detail}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn resolve_git_file(config: &SyncConfig) -> Result<(PathBuf, String)> {
    let dir = PathBuf::from(config.git_repo_dir.trim());
    if !dir.is_dir() {
        bail!("Git sync directory does not exist: {}", dir.display());
    }
    let file_name = config.git_file_name.trim();
    let file_name = if file_name.is_empty() {
        DEFAULT_SYNC_FILENAME
    } else {
        file_name
    };
    if file_name.contains("..") || Path::new(file_name).is_absolute() {
        bail!("invalid sync file name: {file_name}");
    }
    Ok((dir, file_name.to_string()))
}

/// Writes the encrypted snapshot to the configured local Git repo, commits, and pushes.
pub fn git_push_payload(config: &SyncConfig, bundle_json: &str) -> Result<String> {
    let (repo_dir, file_name) = resolve_git_file(config)?;
    let target = repo_dir.join(&file_name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, bundle_json).with_context(|| format!("write {}", target.display()))?;
    run_git(&repo_dir, &["add", "--", &file_name])?;
    // Commit only if the file actually changed.
    let status = run_git(&repo_dir, &["status", "--porcelain", "--", &file_name])?;
    if !status.is_empty() {
        run_git(
            &repo_dir,
            &["commit", "-m", "chore(sync): update encrypted snapshot"],
        )?;
    }
    let remote = if config.git_remote.trim().is_empty() {
        "origin"
    } else {
        config.git_remote.trim()
    };
    let branch = if config.git_branch.trim().is_empty() {
        "main"
    } else {
        config.git_branch.trim()
    };
    run_git(&repo_dir, &["push", remote, branch])?;
    Ok(format!("Pushed encrypted snapshot to {remote}/{branch}"))
}

/// Pulls from the configured Git remote and reads the encrypted snapshot file.
pub fn git_pull_payload(config: &SyncConfig) -> Result<String> {
    let (repo_dir, file_name) = resolve_git_file(config)?;
    let remote = if config.git_remote.trim().is_empty() {
        "origin"
    } else {
        config.git_remote.trim()
    };
    let branch = if config.git_branch.trim().is_empty() {
        "main"
    } else {
        config.git_branch.trim()
    };
    run_git(&repo_dir, &["pull", "--rebase", remote, branch])?;
    let target = repo_dir.join(&file_name);
    fs::read_to_string(&target).with_context(|| format!("read {}", target.display()))
}

/// Pushes an E2E encrypted snapshot using the currently saved [`SyncConfig`].
pub async fn sync_push(passphrase: &str) -> Result<String> {
    let config = load_sync_config()?;
    let bundle = export_encrypted_backup(passphrase)?;
    match config.provider {
        SyncProvider::None => Err(anyhow!("sync provider is not configured")),
        SyncProvider::WebDav => webdav_push_payload(&config, &bundle).await,
        SyncProvider::Git => {
            tokio::task::spawn_blocking(move || git_push_payload(&config, &bundle))
                .await
                .context("git sync push task panicked")?
        }
    }
}

/// Pulls an E2E encrypted snapshot using the currently saved [`SyncConfig`] and imports it.
pub async fn sync_pull(passphrase: &str, replace: bool) -> Result<ImportSummary> {
    let config = load_sync_config()?;
    let bundle = match config.provider {
        SyncProvider::None => bail!("sync provider is not configured"),
        SyncProvider::WebDav => webdav_pull_payload(&config).await?,
        SyncProvider::Git => tokio::task::spawn_blocking(move || git_pull_payload(&config))
            .await
            .context("git sync pull task panicked")??,
    };
    import_encrypted_backup(&bundle, passphrase, replace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::snippets::SnippetScope;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_payload() -> BackupPayload {
        BackupPayload {
            version: 1,
            exported_at: "2026-11-26T00:00:00Z".into(),
            hosts: vec![Host {
                name: "prod".into(),
                hostname: "10.0.0.10".into(),
                user: "deploy".into(),
                password: Some("super-secret".into()),
                ..Host::default()
            }],
            snippets: vec![Snippet {
                name: "Disk usage".into(),
                command: "df -h".into(),
                scope: SnippetScope::Global,
                host: None,
                tags: None,
                params: None,
            }],
            app_config: AppConfig::default(),
        }
    }

    #[test]
    fn backup_encrypts_payload_and_merges_cleanly() {
        let encrypted = encrypt_backup_payload(&sample_payload(), "backup-pw").expect("encrypt");
        assert!(!encrypted.contains("super-secret"));
        assert!(!encrypted.contains("10.0.0.10"));
        assert!(decrypt_backup_payload(&encrypted, "wrong-pw").is_err());

        let decoded = decrypt_backup_payload(&encrypted, "backup-pw").expect("decrypt");
        assert_eq!(decoded.hosts.len(), 1);
        assert_eq!(decoded.hosts[0].password.as_deref(), Some("super-secret"));
        assert_eq!(decoded.snippets.len(), 1);

        let existing = vec![Host {
            name: "other".into(),
            hostname: "10.0.0.2".into(),
            ..Host::default()
        }];
        let merged = merge_backup_hosts(existing.clone(), decoded.hosts.clone(), false);
        assert_eq!(merged.len(), 2);
        let replaced = merge_backup_hosts(existing, decoded.hosts, true);
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].name, "prod");
    }

    #[tokio::test]
    async fn webdav_push_and_pull_roundtrip_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server = tokio::spawn(async move {
            let mut stored = String::new();
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let mut req = vec![0u8; 16384];
                let n = sock.read(&mut req).await.expect("read");
                let text = String::from_utf8_lossy(&req[..n]);
                if text.starts_with("PUT ") {
                    stored = text
                        .split_once("\r\n\r\n")
                        .map(|(_, b)| b.to_string())
                        .unwrap_or_default();
                    sock.write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
                        .await
                        .expect("put resp");
                } else if text.starts_with("GET ") {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                        stored.len(),
                        stored
                    );
                    sock.write_all(resp.as_bytes()).await.expect("get resp");
                }
            }
        });

        let cfg = SyncConfig {
            provider: SyncProvider::WebDav,
            webdav_url: format!("http://127.0.0.1:{port}/dav"),
            webdav_username: "user".into(),
            webdav_password: Some("pass".into()),
            webdav_remote_path: "sync.korabak".into(),
            ..SyncConfig::default()
        };
        let bundle = encrypt_backup_payload(&sample_payload(), "dav-pass").expect("bundle");
        webdav_push_payload(&cfg, &bundle).await.expect("push");
        let pulled = webdav_pull_payload(&cfg).await.expect("pull");
        let decoded = decrypt_backup_payload(&pulled, "dav-pass").expect("decrypt pulled");
        assert_eq!(decoded.hosts[0].name, "prod");
        server.await.expect("server");
    }

    #[test]
    fn git_push_and_pull_roundtrip_against_local_bare_repo() {
        let root = TempDir::new().expect("tempdir");
        let bare = root.path().join("remote.git");
        let work_a = root.path().join("work-a");
        let work_b = root.path().join("work-b");
        fs::create_dir_all(&bare).expect("mkdir bare");
        fs::create_dir_all(&work_a).expect("mkdir a");
        fs::create_dir_all(&work_b).expect("mkdir b");

        run_git(&bare, &["init", "--bare"]).expect("init bare");
        run_git(&work_a, &["init", "-b", "main"]).expect("init a");
        run_git(&work_a, &["config", "user.email", "test@korassh.local"]).expect("email");
        run_git(&work_a, &["config", "user.name", "Test"]).expect("name");
        run_git(
            &work_a,
            &["remote", "add", "origin", bare.to_str().expect("utf8")],
        )
        .expect("add remote");

        let cfg_a = SyncConfig {
            provider: SyncProvider::Git,
            git_repo_dir: work_a.to_string_lossy().into_owned(),
            git_remote: "origin".into(),
            git_branch: "main".into(),
            git_file_name: "vault.korabak".into(),
            ..SyncConfig::default()
        };
        let bundle = encrypt_backup_payload(&sample_payload(), "git-pass").expect("bundle");
        git_push_payload(&cfg_a, &bundle).expect("git push");

        run_git(
            root.path(),
            &[
                "clone",
                "-b",
                "main",
                bare.to_str().expect("utf8"),
                work_b.to_str().expect("utf8"),
            ],
        )
        .expect("clone b");

        let cfg_b = SyncConfig {
            git_repo_dir: work_b.to_string_lossy().into_owned(),
            ..cfg_a
        };
        let pulled = git_pull_payload(&cfg_b).expect("git pull");
        let decoded = decrypt_backup_payload(&pulled, "git-pass").expect("decrypt");
        assert_eq!(decoded.hosts[0].name, "prod");
    }
}
