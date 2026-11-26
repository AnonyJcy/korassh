import { getVersion } from '@tauri-apps/api/app';

// The version baked into the binary: the workspace one, as `kora --version` prints it.
export const installedVersion = (): Promise<string> => getVersion();
