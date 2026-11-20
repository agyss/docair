import { defineConfig, devices } from '@playwright/test';

/**
 * The e2e tests run against a local docair-server (serving the built web app)
 * and a fake AI endpoint, so every run gives the same answers. Set
 * E2E_BASE_URL (e.g. http://localhost:8080, the kind cluster) to test a running
 * deployment instead; the tests that need the fake model are skipped there.
 *
 * The JSON report lands in test-results/results.json; `docair build` puts the
 * results on the test nodes of docair's own docs (docair.toml [tests]).
 */
const external = process.env.E2E_BASE_URL;
const server = {
  command: '../target/release/docair-server',
  url: 'http://127.0.0.1:9556/healthz',
  reuseExistingServer: !process.env.CI,
  env: {
    DOCAIR_BIND: '127.0.0.1:9556',
    DOCAIR_STATIC_DIR: '../web/dist/docair-web/browser',
    DOCAIR_AI_ENDPOINT: 'http://127.0.0.1:9555/v1',
    DOCAIR_AI_MODEL: 'fake-model',
    DOCAIR_AI_API_KEY: 'fake-key',
  },
};
const fakeAi = {
  command: 'node fake-ai.mjs',
  url: 'http://127.0.0.1:9555/health',
  reuseExistingServer: !process.env.CI,
  env: { PORT: '9555' },
};

export default defineConfig({
  testDir: './tests',
  globalSetup: './global-setup.ts',
  fullyParallel: true,
  workers: 4,
  retries: process.env.CI ? 1 : 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [['list'], ['json', { outputFile: 'test-results/results.json' }]],
  use: {
    baseURL: external ?? 'http://127.0.0.1:9556',
    testIdAttribute: 'data-test',
    viewport: { width: 1440, height: 960 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 960 } } }],
  webServer: external ? [fakeAi] : [fakeAi, server],
});
