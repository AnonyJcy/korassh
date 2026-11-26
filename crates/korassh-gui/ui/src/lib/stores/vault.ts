import { writable } from 'svelte/store';
import type { VaultStatusDto } from '$lib/bindings';
import { vaultStatus } from '$lib/ipc/commands';

export const vaultState = writable<VaultStatusDto>({
  mode: 'auto',
  unlocked: true
});

/** Refresh the vault status from the Rust backend (no-op outside Tauri). */
export async function refreshVaultStatus(): Promise<VaultStatusDto> {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    const fallback: VaultStatusDto = { mode: 'auto', unlocked: true };
    vaultState.set(fallback);
    return fallback;
  }
  try {
    const status = await vaultStatus();
    if (status && typeof status === 'object' && 'mode' in status) {
      vaultState.set(status);
      return status;
    }
  } catch {
    // Keep fallback state when outside Tauri or command is unmocked.
  }
  return { mode: 'auto', unlocked: true };
}

/** Format a POSIX mode integer (e.g. 0o755) into symbolic `rwxr-xr-x` + octal `0755`. */
export function formatPermissions(mode: number | null | undefined): string {
  if (mode == null) return '';
  const bits = mode & 0o777;
  const rwx = (v: number) =>
    `${v & 4 ? 'r' : '-'}${v & 2 ? 'w' : '-'}${v & 1 ? 'x' : '-'}`;
  return `${rwx((bits >> 6) & 7)}${rwx((bits >> 3) & 7)}${rwx(bits & 7)}`;
}

export function formatOctalMode(mode: number | null | undefined): string {
  if (mode == null) return '0644';
  return (mode & 0o7777).toString(8).padStart(4, '0');
}
