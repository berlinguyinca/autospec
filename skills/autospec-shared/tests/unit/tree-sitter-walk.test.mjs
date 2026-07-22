// tests/unit/tree-sitter-walk.test.mjs
// Unit tests for skills/autospec-shared/scripts/tree-sitter-walk/walker.mjs
//
// Run: node --test tests/unit/ (from skills/autospec-shared/)
// or:  cd skills/autospec-shared && npm test

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SKILL_DIR = path.resolve(__dirname, '../../');
const FIXTURES_DIR = path.join(SKILL_DIR, 'tests/fixtures/tree-sitter');
const WALKER = path.join(SKILL_DIR, 'scripts/tree-sitter-walk/walker.mjs');

// Import walker
const { walk } = await import(`file://${WALKER}`);

// ── Helpers ───────────────────────────────────────────────────────────────────

function fixture(lang, ext) {
    return path.join(FIXTURES_DIR, lang, `sample.${ext}`);
}

function assertExportExists(output, name) {
    const found = output.exports.find(e => e.name === name);
    assert.ok(found, `Expected export named '${name}' in ${output.file_path}. Got: ${output.exports.map(e => e.name).join(', ')}`);
    return found;
}

function assertImportExists(output, source) {
    const found = output.imports.find(i => i.source === source || i.source.includes(source));
    assert.ok(found, `Expected import from '${source}' in ${output.file_path}. Got: ${output.imports.map(i => i.source).join(', ')}`);
    return found;
}

function assertEntryPointExists(output, kind) {
    const found = output.entry_points.find(e => e.kind === kind);
    assert.ok(found, `Expected entry_point kind='${kind}' in ${output.file_path}. Got: ${JSON.stringify(output.entry_points)}`);
    return found;
}

// ── Malformed input ───────────────────────────────────────────────────────────

test('malformed input: null returns unknown language', async () => {
    const result = await walk(null);
    assert.equal(result.language, 'unknown');
    assert.deepEqual(result.exports, []);
    assert.deepEqual(result.entry_points, []);
    assert.deepEqual(result.imports, []);
});

test('malformed input: nonexistent file returns unknown language', async () => {
    const result = await walk('/tmp/autospec-nonexistent-fixture-xyz.ts');
    assert.equal(result.language, 'unknown');
    assert.deepEqual(result.exports, []);
});

test('malformed input: unsupported extension returns unknown language', async () => {
    // Create a temp .md file
    const tmpFile = path.join('/tmp', 'autospec-test-fixture.md');
    fs.writeFileSync(tmpFile, '# hello\n');
    try {
        const result = await walk(tmpFile);
        assert.equal(result.language, 'unknown');
    } finally {
        fs.unlinkSync(tmpFile);
    }
});

// ── Output schema ─────────────────────────────────────────────────────────────

test('output schema: all required fields present', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    assert.ok(typeof result.language === 'string', 'language must be string');
    assert.ok(Array.isArray(result.exports), 'exports must be array');
    assert.ok(Array.isArray(result.entry_points), 'entry_points must be array');
    assert.ok(Array.isArray(result.imports), 'imports must be array');
    assert.ok(typeof result.file_path === 'string', 'file_path must be string');
});

test('output schema: export entries have required fields', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    for (const exp of result.exports) {
        assert.ok(typeof exp.name === 'string', `export.name must be string, got ${typeof exp.name}`);
        assert.ok(['function', 'class', 'type', 'const'].includes(exp.kind),
            `export.kind must be one of function|class|type|const, got ${exp.kind}`);
        assert.ok(typeof exp.signature === 'string', 'export.signature must be string');
        assert.ok(typeof exp.line === 'number', 'export.line must be number');
    }
});

test('output schema: entry_point entries have required fields', async () => {
    const result = await walk(fixture('go', 'go'));
    for (const ep of result.entry_points) {
        assert.ok(['cli_command', 'http_route'].includes(ep.kind),
            `entry_point.kind must be cli_command|http_route, got ${ep.kind}`);
        assert.ok(typeof ep.identifier === 'string', 'entry_point.identifier must be string');
        assert.ok(typeof ep.line === 'number', 'entry_point.line must be number');
    }
});

test('output schema: import entries have required fields', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    for (const imp of result.imports) {
        assert.ok(typeof imp.source === 'string', 'import.source must be string');
        assert.ok(Array.isArray(imp.names), 'import.names must be array');
    }
});

// ── TypeScript ────────────────────────────────────────────────────────────────

test('typescript: detected language', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    assert.equal(result.language, 'typescript');
});

test('typescript: exports Config interface', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    assertExportExists(result, 'Config');
});

test('typescript: exports parseConfig function', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    const exp = assertExportExists(result, 'parseConfig');
    assert.equal(exp.kind, 'function');
});

test('typescript: exports Server class', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    const exp = assertExportExists(result, 'Server');
    assert.equal(exp.kind, 'class');
});

test('typescript: exports DEFAULT_PORT constant', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    assertExportExists(result, 'DEFAULT_PORT');
});

test('typescript: imports from fs', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    assertImportExists(result, 'fs');
});

test('typescript: file_path is absolute', async () => {
    const result = await walk(fixture('typescript', 'ts'));
    assert.ok(path.isAbsolute(result.file_path), `file_path should be absolute, got: ${result.file_path}`);
});

test('typescript fixture: contains no debug logging sinks', () => {
    const source = fs.readFileSync(fixture('typescript', 'ts'), 'utf8');
    assert.doesNotMatch(source, /console\s*\.\s*(log|debug|info|warn|error)\s*\(/,
        'fixture must not retain console logging');
    assert.doesNotMatch(source, /(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)/,
        'fixture must not retain debugger statements');
});

// ── JavaScript ────────────────────────────────────────────────────────────────

test('javascript: detected language', async () => {
    const result = await walk(fixture('javascript', 'js'));
    assert.equal(result.language, 'javascript');
});

test('javascript: exports greet function', async () => {
    const result = await walk(fixture('javascript', 'js'));
    assertExportExists(result, 'greet');
});

test('javascript: exports VERSION constant', async () => {
    const result = await walk(fixture('javascript', 'js'));
    assertExportExists(result, 'VERSION');
});

test('javascript: detects cli_command entry (shebang)', async () => {
    const result = await walk(fixture('javascript', 'js'));
    assertEntryPointExists(result, 'cli_command');
});

test('javascript: detects http_route entry points', async () => {
    const result = await walk(fixture('javascript', 'js'));
    assertEntryPointExists(result, 'http_route');
});

// ── Python ────────────────────────────────────────────────────────────────────

test('python: detected language', async () => {
    const result = await walk(fixture('python', 'py'));
    assert.equal(result.language, 'python');
});

test('python: exports parse_config function', async () => {
    const result = await walk(fixture('python', 'py'));
    assertExportExists(result, 'parse_config');
});

test('python: exports ConfigLoader class', async () => {
    const result = await walk(fixture('python', 'py'));
    assertExportExists(result, 'ConfigLoader');
});

test('python: detects cli_command entry (__main__)', async () => {
    const result = await walk(fixture('python', 'py'));
    assertEntryPointExists(result, 'cli_command');
});

test('python: imports from os', async () => {
    const result = await walk(fixture('python', 'py'));
    assertImportExists(result, 'os');
});

// ── Go ────────────────────────────────────────────────────────────────────────

test('go: detected language', async () => {
    const result = await walk(fixture('go', 'go'));
    assert.equal(result.language, 'go');
});

test('go: exports ParseConfig function', async () => {
    const result = await walk(fixture('go', 'go'));
    assertExportExists(result, 'ParseConfig');
});

test('go: exports FormatAddress function', async () => {
    const result = await walk(fixture('go', 'go'));
    assertExportExists(result, 'FormatAddress');
});

test('go: detects cli_command (main function)', async () => {
    const result = await walk(fixture('go', 'go'));
    assertEntryPointExists(result, 'cli_command');
});

test('go: detects http_route entry point', async () => {
    const result = await walk(fixture('go', 'go'));
    assertEntryPointExists(result, 'http_route');
});

test('go: imports from net/http', async () => {
    const result = await walk(fixture('go', 'go'));
    assertImportExists(result, 'net/http');
});

// ── Rust ──────────────────────────────────────────────────────────────────────

test('rust: detected language', async () => {
    const result = await walk(fixture('rust', 'rs'));
    assert.equal(result.language, 'rust');
});

test('rust: exports parse_config function', async () => {
    const result = await walk(fixture('rust', 'rs'));
    assertExportExists(result, 'parse_config');
});

test('rust: exports Config struct (type)', async () => {
    const result = await walk(fixture('rust', 'rs'));
    assertExportExists(result, 'Config');
});

test('rust: exports Handler trait', async () => {
    const result = await walk(fixture('rust', 'rs'));
    assertExportExists(result, 'Handler');
});

test('rust: detects cli_command (main function)', async () => {
    const result = await walk(fixture('rust', 'rs'));
    assertEntryPointExists(result, 'cli_command');
});

test('rust: imports from std::fs', async () => {
    const result = await walk(fixture('rust', 'rs'));
    assertImportExists(result, 'fs');
});

// ── Java ──────────────────────────────────────────────────────────────────────

test('java: detected language', async () => {
    const result = await walk(fixture('java', 'java'));
    assert.equal(result.language, 'java');
});

test('java: exports ConfigLoader class', async () => {
    const result = await walk(fixture('java', 'java'));
    assertExportExists(result, 'ConfigLoader');
});

test('java: exports loadConfig method (function)', async () => {
    const result = await walk(fixture('java', 'java'));
    assertExportExists(result, 'loadConfig');
});

test('java: detects cli_command (main method)', async () => {
    const result = await walk(fixture('java', 'java'));
    assertEntryPointExists(result, 'cli_command');
});

test('java: imports java.io.IOException', async () => {
    const result = await walk(fixture('java', 'java'));
    assert.ok(result.imports.length > 0, 'Expected at least one import');
});
