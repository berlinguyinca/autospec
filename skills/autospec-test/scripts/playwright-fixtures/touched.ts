// touched.ts — Playwright fixture that records (route, selector) interactions.
//
// Wraps Playwright's Page interaction APIs (click, fill, selectOption, etc.)
// and appends JSONL records to .autospec/touched-elements.jsonl in the test repo.
//
// Usage: import { test } from './touched';
//
// Each record:
//   { "route": "/dashboard", "selector": "[data-testid=submit-btn]", "action": "click", "ts": 1234567890 }

import { test as base, Page, expect } from '@playwright/test';
import { appendFileSync, mkdirSync, existsSync } from 'fs';
import { join } from 'path';

const TOUCHED_LOG = join(process.cwd(), '.autospec', 'touched-elements.jsonl');

function ensureDir() {
  const dir = join(process.cwd(), '.autospec');
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
}

function record(route: string, selector: string, action: string) {
  ensureDir();
  const entry = JSON.stringify({ route, selector, action, ts: Date.now() });
  appendFileSync(TOUCHED_LOG, entry + '\n', 'utf8');
}

// Extend base test with a recording page fixture
export const test = base.extend<{ recordingPage: Page }>({
  recordingPage: async ({ page }, use) => {
    // Patch interaction methods to record (route, selector)
    const originalClick = page.click.bind(page);
    page.click = async (selector: string, options?: Parameters<Page['click']>[1]) => {
      const route = new URL(page.url()).pathname;
      record(route, selector, 'click');
      return originalClick(selector, options);
    };

    const originalFill = page.fill.bind(page);
    page.fill = async (selector: string, value: string, options?: Parameters<Page['fill']>[2]) => {
      const route = new URL(page.url()).pathname;
      record(route, selector, 'fill');
      return originalFill(selector, value, options);
    };

    const originalSelectOption = page.selectOption.bind(page);
    page.selectOption = async (selector: string, values: Parameters<Page['selectOption']>[1], options?: Parameters<Page['selectOption']>[2]) => {
      const route = new URL(page.url()).pathname;
      record(route, selector, 'selectOption');
      return originalSelectOption(selector, values, options);
    };

    await use(page);
  },
});

export { expect };
