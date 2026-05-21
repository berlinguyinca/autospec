// playwright.config.js — minimal JS fixture for config resolver tests
module.exports = {
  use: {
    baseURL: 'http://localhost:3000',
  },
  webServer: {
    url: 'http://localhost:3000',
    command: 'npm start',
  },
  testDir: './tests/e2e',
  projects: [
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
};
