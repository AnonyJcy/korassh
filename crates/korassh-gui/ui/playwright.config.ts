import { defineConfig, devices } from '@playwright/test';

// e2e runs against the built static SPA (adapter-static) served by `vite preview`,
// which matches how Tauri serves the bundle (tech-gui.md §6.4). Tauri APIs are absent
// off-runtime, so flows exercise the localStorage-mirrored persistence; the single
// bundle-level launch smoke (tauri-driver) is reserved for Stage 5.
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:4173',
    trace: 'on-first-retry',
    // The product defaults to zh-CN when nothing is saved. The spec files inherited
    // from upstream are written against the English UI, so every context starts with
    // English saved. A spec that sets the locale in its own init script still wins:
    // init scripts run after the stored state is restored.
    storageState: {
      cookies: [],
      origins: [
        {
          origin: 'http://localhost:4173',
          localStorage: [{ name: 'korassh-locale', value: 'en' }]
        }
      ]
    }
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'npm run build && npm run preview -- --port 4173 --strictPort',
    url: 'http://localhost:4173',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000
  }
});
