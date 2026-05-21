// playwright.config.ts — TypeScript fixture for config resolver tests
import { defineConfig } from '@playwright/test';

export default defineConfig({
  use: {
    baseURL: 'http://localhost:4000',
  },
  webServer: {
    url: 'http://localhost:4000',
    command: 'npm run dev',
  },
  testDir: './e2e',
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
    { name: 'firefox', use: { browserName: 'firefox' } },
  ],
});
