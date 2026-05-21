// scripts/playwright-fixtures/touched.ts
// Playwright fixture that wraps interaction APIs to record (route, selector) pairs.
//
// Wrapped interactions per spec §4 Metric B and AC (PR #331 finding #6 fix):
//   click, fill, selectOption, keyboard, drag, scroll, upload
//
// Records to .autospec/touched-elements.jsonl in the target repo root.
// Each line: {"route":"<url>","selector":"<selector>","interaction":"<type>","ts":<ms>}

import { test as base, type Page, type Locator } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';

const TOUCHED_FILE = path.join(process.cwd(), '.autospec', 'touched-elements.jsonl');

/**
 * Append a touched-element record to the JSONL file.
 * Uses synchronous write to avoid race conditions between parallel workers.
 */
function recordTouched(route: string, selector: string, interaction: string): void {
    const dir = path.dirname(TOUCHED_FILE);
    if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
    }
    const entry = JSON.stringify({ route, selector, interaction, ts: Date.now() }) + '\n';
    fs.appendFileSync(TOUCHED_FILE, entry, 'utf8');
}

/**
 * Get the best stable selector string for a Locator.
 * Falls back to the locator's toString() which includes the selector expression.
 */
function selectorOf(locator: Locator): string {
    // Playwright's Locator.toString() returns something like "Locator@<selector>"
    const str = locator.toString();
    const match = str.match(/Locator@(.+)$/);
    return match ? match[1] : str;
}

/** Autospec-enhanced page type with all interaction wrappers. */
export type AutospecPage = Page & {
    autospecClick(locator: Locator, options?: Parameters<Locator['click']>[0]): Promise<void>;
    autospecFill(locator: Locator, value: string, options?: Parameters<Locator['fill']>[0]): Promise<void>;
    autospecSelectOption(locator: Locator, values: Parameters<Locator['selectOption']>[0], options?: Parameters<Locator['selectOption']>[1]): Promise<string[]>;
    autospecKeyboard(key: string): Promise<void>;
    autospecDragTo(source: Locator, target: Locator, options?: Parameters<Locator['dragTo']>[0]): Promise<void>;
    autospecScroll(locator: Locator, deltaX: number, deltaY: number): Promise<void>;
    autospecUpload(locator: Locator, files: Parameters<Locator['setInputFiles']>[0], options?: Parameters<Locator['setInputFiles']>[1]): Promise<void>;
};

/**
 * autospecPage fixture — wraps all required interaction methods.
 * Use this in tests instead of `page` when coverage tracking is needed.
 */
export const test = base.extend<{ autospecPage: AutospecPage }>({
    autospecPage: async ({ page }, use) => {
        const aPage = page as AutospecPage;

        /** Wrapped click — records (route, selector) */
        aPage.autospecClick = async (locator, options?) => {
            const route = page.url();
            const selector = selectorOf(locator);
            await locator.click(options);
            recordTouched(route, selector, 'click');
        };

        /** Wrapped fill — records (route, selector) */
        aPage.autospecFill = async (locator, value, options?) => {
            const route = page.url();
            const selector = selectorOf(locator);
            await locator.fill(value, options);
            recordTouched(route, selector, 'fill');
        };

        /** Wrapped selectOption — records (route, selector) */
        aPage.autospecSelectOption = async (locator, values, options?) => {
            const route = page.url();
            const selector = selectorOf(locator);
            const result = await locator.selectOption(values, options);
            recordTouched(route, selector, 'selectOption');
            return result;
        };

        /** Wrapped keyboard press — records (route, key) */
        aPage.autospecKeyboard = async (key) => {
            const route = page.url();
            await page.keyboard.press(key);
            recordTouched(route, `keyboard:${key}`, 'keyboard');
        };

        /** Wrapped drag-to — records (route, source→target) */
        aPage.autospecDragTo = async (source, target, options?) => {
            const route = page.url();
            const sourceSelector = selectorOf(source);
            const targetSelector = selectorOf(target);
            await source.dragTo(target, options);
            recordTouched(route, `${sourceSelector}→${targetSelector}`, 'drag');
        };

        /** Wrapped scroll (mouse wheel) — records (route, selector) */
        aPage.autospecScroll = async (locator, deltaX, deltaY) => {
            const route = page.url();
            const selector = selectorOf(locator);
            const box = await locator.boundingBox();
            if (box) {
                await page.mouse.wheel(deltaX, deltaY);
            }
            recordTouched(route, selector, 'scroll');
        };

        /** Wrapped file upload (setInputFiles) — records (route, selector) */
        aPage.autospecUpload = async (locator, files, options?) => {
            const route = page.url();
            const selector = selectorOf(locator);
            await locator.setInputFiles(files, options);
            recordTouched(route, selector, 'upload');
        };

        await use(aPage);
    },
});

export { expect } from '@playwright/test';
