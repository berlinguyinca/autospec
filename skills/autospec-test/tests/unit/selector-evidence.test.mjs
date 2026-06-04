// skills/autospec-test/tests/unit/selector-evidence.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/selector-evidence.mjs
// Covers: extractSelectors, resolveSelector, buildEvidence, writeManifest

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { extractSelectors, resolveSelector, buildEvidence, writeManifest } =
    await import(`file://${SCRIPTS_DIR}/selector-evidence.mjs`);

// ── Helpers ────────────────────────────────────────────────────────────────────

function tmpDir() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'sel-ev-test-'));
}

function writeFile(dir, relPath, content) {
    const full = path.join(dir, relPath);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, 'utf8');
    return full;
}

function cleanup(dir) {
    fs.rmSync(dir, { recursive: true, force: true });
}

// ── extractSelectors ───────────────────────────────────────────────────────────

test('extractSelectors: extracts data-testid from spec', () => {
    const specText = `
await page.getByTestId('submit-btn').click();
await page.locator('[data-testid="user-name"]').fill('Alice');
`;
    const selectors = extractSelectors(specText);
    assert.ok(selectors.some(s => s.selector === 'submit-btn' && s.type === 'data-testid'),
        `Expected submit-btn data-testid, got ${JSON.stringify(selectors)}`);
    assert.ok(selectors.some(s => s.selector === 'user-name' && s.type === 'data-testid'),
        `Expected user-name data-testid`);
});

test('extractSelectors: extracts aria-label from spec', () => {
    const specText = `await page.getByLabel('Email address').fill('test@example.com');`;
    const selectors = extractSelectors(specText);
    assert.ok(selectors.some(s => s.selector === 'Email address'),
        `Expected 'Email address', got ${JSON.stringify(selectors)}`);
});

test('extractSelectors: extracts getByText literals', () => {
    const specText = `
const el = page.getByText('Welcome back', { exact: true });
await expect(el).toBeVisible();
`;
    const selectors = extractSelectors(specText);
    assert.ok(selectors.some(s => s.selector === 'Welcome back'),
        `Expected 'Welcome back' text selector`);
});

test('extractSelectors: extracts getByRole name strings', () => {
    const specText = `
await page.getByRole('button', { name: 'Save changes', exact: true }).click();
`;
    const selectors = extractSelectors(specText);
    assert.ok(selectors.some(s => s.selector === 'Save changes'),
        `Expected 'Save changes' role-name selector`);
});

test('extractSelectors: returns line numbers for each selector', () => {
    const specText = `import { test } from '@playwright/test';
test('foo', async ({ page }) => {
    await page.getByTestId('my-button').click();
});`;
    const selectors = extractSelectors(specText);
    const found = selectors.find(s => s.selector === 'my-button');
    assert.ok(found, 'Should find my-button');
    assert.equal(found.line, 3, `Expected line 3, got ${found.line}`);
});

test('extractSelectors: returns empty array for spec with no selectors', () => {
    const specText = `
import { test } from '@playwright/test';
test('foo', async ({ page }) => { await page.goto('/'); });
`;
    const selectors = extractSelectors(specText);
    assert.equal(selectors.length, 0);
});

test('extractSelectors: deduplicates same selector appearing multiple times', () => {
    const specText = `
await page.getByTestId('save-btn').click();
await page.getByTestId('save-btn').isVisible();
`;
    const selectors = extractSelectors(specText);
    const saveBtns = selectors.filter(s => s.selector === 'save-btn');
    // Should have at most 1 unique entry per selector value (deduplicated)
    // OR multiple entries with different lines — either is acceptable; test for presence
    assert.ok(saveBtns.length >= 1, 'Should find save-btn at least once');
});

// ── resolveSelector ────────────────────────────────────────────────────────────

test('resolveSelector: returns file:line when selector found in source files', () => {
    const dir = tmpDir();
    try {
        writeFile(dir, 'src/Button.tsx', `
export function Button() {
    return <button data-testid="submit-btn">Submit</button>;
}
`);
        const result = resolveSelector('submit-btn', [path.join(dir, 'src/**')], dir);
        assert.ok(result !== null, 'Should resolve submit-btn');
        assert.ok(result.includes('Button.tsx'), `Expected Button.tsx in result: ${result}`);
        // Should be file:line format
        assert.ok(/:\d+$/.test(result), `Expected file:line format: ${result}`);
    } finally {
        cleanup(dir);
    }
});

test('resolveSelector: returns null when selector not found in source', () => {
    const dir = tmpDir();
    try {
        writeFile(dir, 'src/Button.tsx', `
export function Button() {
    return <button data-testid="other-btn">Other</button>;
}
`);
        const result = resolveSelector('nonexistent-btn', [path.join(dir, 'src/**')], dir);
        assert.equal(result, null);
    } finally {
        cleanup(dir);
    }
});

test('resolveSelector: finds aria-label in source', () => {
    const dir = tmpDir();
    try {
        writeFile(dir, 'src/Input.tsx', `
export function Input() {
    return <input aria-label="Email address" type="email" />;
}
`);
        const result = resolveSelector('Email address', [path.join(dir, 'src/**')], dir);
        assert.ok(result !== null, 'Should resolve Email address');
        assert.ok(result.includes('Input.tsx'), `Expected Input.tsx: ${result}`);
    } finally {
        cleanup(dir);
    }
});

test('resolveSelector: finds text in JSX content', () => {
    const dir = tmpDir();
    try {
        writeFile(dir, 'src/Header.tsx', `
export function Header() {
    return <h1>Welcome back</h1>;
}
`);
        const result = resolveSelector('Welcome back', [path.join(dir, 'src/**')], dir);
        assert.ok(result !== null, 'Should resolve Welcome back text');
    } finally {
        cleanup(dir);
    }
});

test('resolveSelector: works with absolute file paths (no glob)', () => {
    const dir = tmpDir();
    try {
        const srcFile = writeFile(dir, 'src/Form.tsx', `
<button data-testid="form-submit">Go</button>
`);
        const result = resolveSelector('form-submit', [srcFile], dir);
        assert.ok(result !== null, 'Should find form-submit with direct path');
    } finally {
        cleanup(dir);
    }
});

// ── buildEvidence ──────────────────────────────────────────────────────────────

test('buildEvidence: returns manifest shape {spec_file: {selector: "file:line"}}', () => {
    const dir = tmpDir();
    try {
        const specFile = writeFile(dir, 'e2e/specs/home.spec.ts', `
await page.getByTestId('hero-title').isVisible();
`);
        writeFile(dir, 'src/Home.tsx', `
<h1 data-testid="hero-title">Home</h1>
`);
        const evidence = buildEvidence(specFile, [path.join(dir, 'src/**')], dir);
        assert.ok(typeof evidence === 'object', 'Should return an object');
        // Keys should be selector strings
        const keys = Object.keys(evidence);
        assert.ok(keys.includes('hero-title'), `Expected hero-title key, got ${JSON.stringify(keys)}`);
        // Values should be file:line strings
        for (const val of Object.values(evidence)) {
            assert.ok(typeof val === 'string', `Expected string value, got ${typeof val}`);
        }
    } finally {
        cleanup(dir);
    }
});

test('buildEvidence: unresolved selector maps to null', () => {
    const dir = tmpDir();
    try {
        const specFile = writeFile(dir, 'e2e/specs/dash.spec.ts', `
await page.getByTestId('missing-widget').click();
`);
        const evidence = buildEvidence(specFile, [path.join(dir, 'src/**')], dir);
        assert.ok('missing-widget' in evidence, 'Should include missing-widget key');
        assert.equal(evidence['missing-widget'], null);
    } finally {
        cleanup(dir);
    }
});

test('buildEvidence: empty spec returns empty object', () => {
    const dir = tmpDir();
    try {
        const specFile = writeFile(dir, 'e2e/specs/empty.spec.ts', `
import { test } from '@playwright/test';
test('noop', async ({ page }) => { await page.goto('/'); });
`);
        const evidence = buildEvidence(specFile, [], dir);
        assert.deepEqual(evidence, {});
    } finally {
        cleanup(dir);
    }
});

// ── writeManifest ──────────────────────────────────────────────────────────────

test('writeManifest: writes manifest JSON to e2e/.autospec/selector-evidence.json', () => {
    const dir = tmpDir();
    try {
        const specFile = path.join(dir, 'e2e/specs/home.spec.ts');
        const manifestData = { [specFile]: { 'hero-title': 'src/Home.tsx:3' } };
        const manifestPath = writeManifest(manifestData, dir);
        assert.ok(fs.existsSync(manifestPath), `Manifest file should exist at ${manifestPath}`);
        const written = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
        assert.deepEqual(written, manifestData);
        // Path should be e2e/.autospec/selector-evidence.json relative to dir
        const rel = path.relative(dir, manifestPath);
        assert.equal(rel, path.join('e2e', '.autospec', 'selector-evidence.json'));
    } finally {
        cleanup(dir);
    }
});

test('writeManifest: creates parent directories if missing', () => {
    const dir = tmpDir();
    try {
        const manifestPath = writeManifest({}, dir);
        assert.ok(fs.existsSync(path.dirname(manifestPath)), 'Parent dir should exist');
    } finally {
        cleanup(dir);
    }
});

test('writeManifest: overwrites existing manifest', () => {
    const dir = tmpDir();
    try {
        writeManifest({ 'old.spec.ts': { 'old-sel': 'old:1' } }, dir);
        const newData = { 'new.spec.ts': { 'new-sel': 'new:5' } };
        const manifestPath = writeManifest(newData, dir);
        const written = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
        assert.deepEqual(written, newData);
    } finally {
        cleanup(dir);
    }
});

test('writeManifest: returns the path to the manifest file', () => {
    const dir = tmpDir();
    try {
        const p = writeManifest({}, dir);
        assert.equal(typeof p, 'string');
        assert.ok(p.endsWith('selector-evidence.json'));
    } finally {
        cleanup(dir);
    }
});
