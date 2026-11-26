//! Local encryption vault commands: status, unlock, lock, set/change/remove master password.

use omnyssh_core::config::vault;

use crate::dto::VaultStatusDto;
use crate::error::CommandError;

/// Return the current vault protection mode and unlock state.
#[tauri::command]
#[specta::specta]
pub fn vault_status() -> VaultStatusDto {
    vault::status().into()
}

/// Unlock a master-password-protected vault into memory.
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock(master_password: String) -> Result<VaultStatusDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        vault::unlock(&master_password).map_err(|e| format!("{e:#}"))?;
        Ok::<VaultStatusDto, String>(vault::status().into())
    })
    .await
    .map_err(|e| CommandError {
        message: format!("vault unlock task failed: {e}"),
    })?
    .map_err(|message| CommandError { message })
}

/// Clear the decrypted key from memory (locks a master-password vault).
#[tauri::command]
#[specta::specta]
pub fn vault_lock() -> VaultStatusDto {
    vault::lock();
    vault::status().into()
}

/// Enable or change the master password (Argon2id + AES-256-GCM key wrapping).
#[tauri::command]
#[specta::specta]
pub async fn vault_set_master_password(
    current_password: Option<String>,
    new_password: String,
) -> Result<VaultStatusDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        vault::set_master_password(current_password.as_deref(), &new_password)
            .map_err(|e| format!("{e:#}"))?;
        Ok::<VaultStatusDto, String>(vault::status().into())
    })
    .await
    .map_err(|e| CommandError {
        message: format!("vault task failed: {e}"),
    })?
    .map_err(|message| CommandError { message })
}

/// Verify the current master password and return the vault to seamless Auto encryption mode.
#[tauri::command]
#[specta::specta]
pub async fn vault_remove_master_password(
    current_password: String,
) -> Result<VaultStatusDto, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        vault::remove_master_password(&current_password).map_err(|e| format!("{e:#}"))?;
        Ok::<VaultStatusDto, String>(vault::status().into())
    })
    .await
    .map_err(|e| CommandError {
        message: format!("vault task failed: {e}"),
    })?
    .map_err(|message| CommandError { message })
}
