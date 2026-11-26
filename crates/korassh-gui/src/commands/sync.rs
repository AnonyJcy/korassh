//! End-to-end encrypted backup import/export and WebDAV / Git sync commands.

use korassh_core::config::sync::{self, SyncConfig};

use crate::dto::{ImportSummaryDto, SyncConfigDto, SyncConfigInputDto};
use crate::error::CommandError;

/// Export hosts, snippets, and app config into a passphrase-encrypted `.korabak` JSON string.
#[tauri::command]
#[specta::specta]
pub async fn export_backup(passphrase: String) -> Result<String, CommandError> {
    tauri::async_runtime::spawn_blocking(move || sync::export_encrypted_backup(&passphrase))
        .await
        .map_err(|e| CommandError {
            message: format!("backup export task failed: {e}"),
        })?
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })
}

/// Decrypt and import an encrypted `.korabak` JSON bundle (merge or replace).
#[tauri::command]
#[specta::specta]
pub async fn import_backup(
    bundle_json: String,
    passphrase: String,
    replace: bool,
) -> Result<ImportSummaryDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        sync::import_encrypted_backup(&bundle_json, &passphrase, replace)
    })
    .await
    .map_err(|e| CommandError {
        message: format!("backup import task failed: {e}"),
    })?
    .map(ImportSummaryDto::from)
    .map_err(|e| CommandError {
        message: format!("{e:#}"),
    })
}

/// Load persisted WebDAV / Git sync configuration (WebDAV password omitted).
#[tauri::command]
#[specta::specta]
pub async fn load_sync_config() -> Result<SyncConfigDto, CommandError> {
    tauri::async_runtime::spawn_blocking(sync::load_sync_config)
        .await
        .map_err(|e| CommandError {
            message: format!("sync config load task failed: {e}"),
        })?
        .map(|cfg| SyncConfigDto::from(&cfg))
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })
}

/// Persist WebDAV / Git sync configuration, preserving the stored WebDAV password when omitted.
#[tauri::command]
#[specta::specta]
pub async fn save_sync_config(input: SyncConfigInputDto) -> Result<SyncConfigDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let existing = sync::load_sync_config().unwrap_or_default();
        let next_pw = input
            .webdav_password
            .filter(|p| !p.is_empty())
            .or(existing.webdav_password);
        let cfg = SyncConfig {
            provider: input.provider.into(),
            webdav_url: input.webdav_url.trim().to_string(),
            webdav_username: input.webdav_username.trim().to_string(),
            webdav_password: next_pw,
            webdav_remote_path: input.webdav_remote_path.trim().to_string(),
            git_repo_dir: input.git_repo_dir.trim().to_string(),
            git_remote: input.git_remote.trim().to_string(),
            git_branch: input.git_branch.trim().to_string(),
            git_file_name: input.git_file_name.trim().to_string(),
        };
        sync::save_sync_config(&cfg).map_err(|e| format!("{e:#}"))?;
        Ok::<SyncConfigDto, String>(SyncConfigDto::from(&cfg))
    })
    .await
    .map_err(|e| CommandError {
        message: format!("sync config save task failed: {e}"),
    })?
    .map_err(|message| CommandError { message })
}

/// Encrypt and push the current snapshot to the configured WebDAV or Git remote.
#[tauri::command]
#[specta::specta]
pub async fn sync_push(passphrase: String) -> Result<String, CommandError> {
    sync::sync_push(&passphrase)
        .await
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })
}

/// Pull and decrypt the latest snapshot from the configured WebDAV or Git remote.
#[tauri::command]
#[specta::specta]
pub async fn sync_pull(
    passphrase: String,
    replace: bool,
) -> Result<ImportSummaryDto, CommandError> {
    sync::sync_pull(&passphrase, replace)
        .await
        .map(ImportSummaryDto::from)
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })
}
