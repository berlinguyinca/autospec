// tests/unit/scan-doc-scope.test.mjs
// Unit tests for skills/autospec-shared/scripts/scan-doc-scope.mjs
//
// Run: node tests/unit/scan-doc-scope.test.mjs (from skills/autospec-shared/)
// or:  npm test  (if test script is updated)

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SKILL_DIR = path.resolve(__dirname, '../../');
const FIXTURES_DIR = path.join(SKILL_DIR, 'tests/fixtures/doc-scope');
const SCAN_MJS = path.join(SKILL_DIR, 'scripts/scan-doc-scope.mjs');

const { parse } = await import(`file://${SCAN_MJS}`);

// ── Helpers ───────────────────────────────────────────────────────────────────

function fixture(name) {
    return path.join(FIXTURES_DIR, name);
}

// ── Error cases ───────────────────────────────────────────────────────────────

test('throws on null input', async () => {
    assert.throws(() => parse(null), /markdownPath must be a non-empty string/);
});

test('throws on missing file', async () => {
    assert.throws(
        () => parse('/tmp/autospec-nonexistent-fixture-xyz.md'),
        /cannot read file/
    );
});

test('throws on malformed YAML in scope block', async () => {
    assert.throws(
        () => parse(fixture('malformed-yaml.md')),
        /scan-doc-scope/
    );
});

// ── No scope ──────────────────────────────────────────────────────────────────

test('returns empty array for file with no scope declarations', async () => {
    const result = parse(fixture('no-scope.md'));
    assert.ok(Array.isArray(result));
    assert.equal(result.length, 0);
});

// ── Valid scope ───────────────────────────────────────────────────────────────

test('parses multiple scope blocks from valid-scope.md', async () => {
    const result = parse(fixture('valid-scope.md'));
    assert.ok(Array.isArray(result));
    assert.equal(result.length, 3, `Expected 3 scope entries, got ${result.length}`);
});

test('first entry: heading_path is the Installing heading', async () => {
    const result = parse(fixture('valid-scope.md'));
    assert.ok(result[0].heading_path.includes('Installing'), `heading_path: ${result[0].heading_path}`);
});

test('first entry: src_globs contains both install.sh globs', async () => {
    const result = parse(fixture('valid-scope.md'));
    assert.ok(Array.isArray(result[0].src_globs));
    assert.equal(result[0].src_globs.length, 2);
    assert.ok(result[0].src_globs.includes('install.sh'));
    assert.ok(result[0].src_globs.includes('skills/autospec-test/install.sh'));
});

test('first entry: reason is present', async () => {
    const result = parse(fixture('valid-scope.md'));
    assert.ok(typeof result[0].reason === 'string');
    assert.ok(result[0].reason.length > 0);
});

test('second entry: has visual_glob', async () => {
    const result = parse(fixture('valid-scope.md'));
    assert.ok(result[1].visual_glob, `Expected visual_glob in entry 1: ${JSON.stringify(result[1])}`);
    assert.ok(result[1].visual_glob.includes('screenshots'));
});

test('third entry: generated is true', async () => {
    const result = parse(fixture('valid-scope.md'));
    assert.equal(result[2].generated, true);
});

test('all entries: byte_range is a two-element number array', async () => {
    const result = parse(fixture('valid-scope.md'));
    for (const entry of result) {
        assert.ok(Array.isArray(entry.byte_range), 'byte_range must be array');
        assert.equal(entry.byte_range.length, 2, 'byte_range must have 2 elements');
        assert.ok(typeof entry.byte_range[0] === 'number', 'byte_range[0] must be number');
        assert.ok(typeof entry.byte_range[1] === 'number', 'byte_range[1] must be number');
        assert.ok(entry.byte_range[0] <= entry.byte_range[1], 'byte_range start must be <= end');
    }
});

test('all entries: src_globs is always an array', async () => {
    const result = parse(fixture('valid-scope.md'));
    for (const entry of result) {
        assert.ok(Array.isArray(entry.src_globs), `src_globs must be array in ${entry.heading_path}`);
    }
});

// ── Generated section ─────────────────────────────────────────────────────────

test('generated-section.md: parses generated:true entry', async () => {
    const result = parse(fixture('generated-section.md'));
    assert.equal(result.length, 1);
    assert.equal(result[0].generated, true);
    assert.ok(result[0].src_globs.some(g => g.includes('**')), 'should have glob pattern');
});

// ── Fenced code block awareness (#561) ─────────────────────────────────────────

test('fenced-scope.md: only prose declarations are honored, in-fence tokens ignored', async () => {
    const result = parse(fixture('fenced-scope.md'));
    const globs = result.flatMap(e => e.src_globs);
    // In-fence tokens must NOT appear
    assert.ok(!globs.includes('scripts/example-in-fence.sh'), 'token inside ``` fence must be ignored');
    assert.ok(!globs.includes('scripts/another-in-fence.sh'), 'token inside language-tagged fence must be ignored');
    assert.ok(!globs.includes('scripts/tilde-in-fence.sh'), 'token inside ~~~ fence must be ignored');
    // Prose declarations (before and after fences) must be honored
    assert.ok(globs.includes('scripts/real.sh'), 'prose declaration before fence must be honored');
    assert.ok(globs.includes('scripts/post-fence.sh'), 'prose declaration after closed fences must be honored');
    assert.equal(result.length, 2, `expected exactly 2 honored entries, got ${result.length}`);
});

test('scope token inside a fenced block is ignored', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-fence-${Date.now()}.md`);
    fs.writeFileSync(tmp, `## Section\n\n\`\`\`\n<!-- autospec-doc-scope:\n  src: ["in-fence.sh"]\n-->\n\`\`\`\n\nBody.\n`);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 0, 'in-fence declaration must yield no entries');
    } finally {
        fs.unlinkSync(tmp);
    }
});

test('scope token after a closed fence is honored', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-postfence-${Date.now()}.md`);
    fs.writeFileSync(tmp, `## Section\n\n\`\`\`bash\necho hi\n\`\`\`\n\n<!-- autospec-doc-scope:\n  src: ["after.sh"]\n-->\n\nBody.\n`);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 1);
        assert.deepEqual(result[0].src_globs, ['after.sh']);
    } finally {
        fs.unlinkSync(tmp);
    }
});

test('unclosed trailing fence swallows declarations inside it', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-unclosed-${Date.now()}.md`);
    fs.writeFileSync(tmp, `## Section\n\n\`\`\`\n<!-- autospec-doc-scope:\n  src: ["trailing.sh"]\n-->\n`);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 0, 'declaration inside an unclosed trailing fence must be ignored');
    } finally {
        fs.unlinkSync(tmp);
    }
});

// ── Inline fixture via temp file ──────────────────────────────────────────────

test('single-glob src in flow syntax', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-${Date.now()}.md`);
    fs.writeFileSync(tmp, `## My Section\n\n<!-- autospec-doc-scope:\n  src: ["path/to/file.sh"]\n-->\n\nBody.\n`);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 1);
        assert.deepEqual(result[0].src_globs, ['path/to/file.sh']);
    } finally {
        fs.unlinkSync(tmp);
    }
});

test('scope block with no src field yields empty src_globs', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-${Date.now()}.md`);
    fs.writeFileSync(tmp, `## Section\n\n<!-- autospec-doc-scope:\n  reason: "no src"\n-->\n\nBody.\n`);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 1);
        assert.deepEqual(result[0].src_globs, []);
    } finally {
        fs.unlinkSync(tmp);
    }
});

test('scope block on same line as closing -->', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-${Date.now()}.md`);
    fs.writeFileSync(tmp, `## Section\n\n<!-- autospec-doc-scope:\n  src: ["a.sh"]\n  reason: "test"\n-->\n\nBody.\n`);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 1);
        assert.deepEqual(result[0].src_globs, ['a.sh']);
    } finally {
        fs.unlinkSync(tmp);
    }
});

test('byte_range start is after the scope comment', async () => {
    const tmp = path.join(os.tmpdir(), `autospec-test-scope-${Date.now()}.md`);
    const content = `## Section\n\n<!-- autospec-doc-scope:\n  src: ["a.sh"]\n-->\n\nBody text here.\n`;
    fs.writeFileSync(tmp, content);
    try {
        const result = parse(tmp);
        assert.equal(result.length, 1);
        const [start, end] = result[0].byte_range;
        assert.ok(start > 0, 'byte_range start should be > 0');
        assert.ok(end > start, 'byte_range end should be > start');
        // The body "Body text here." should fall within the range
        const body = content.slice(start, end);
        assert.ok(body.includes('Body'), `body should contain 'Body', got: ${JSON.stringify(body)}`);
    } finally {
        fs.unlinkSync(tmp);
    }
});
