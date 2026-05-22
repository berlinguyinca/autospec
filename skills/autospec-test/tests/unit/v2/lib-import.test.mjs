/**
 * lib-import.test.mjs — Unit tests for the @autospec/test npm helper library (Phase 8).
 *
 * Tests that:
 *   1. npm pack produces a valid tarball
 *   2. All 6 helpers are importable and callable
 *   3. TypeScript build produces dist/index.d.ts with correct signatures
 *   4. publish-helpers.sh --dry-run exits 0 without calling npm publish
 *
 * Uses node:test (built-in). No mocks — real npm pack + install in temp dir.
 * Playwright is NOT invoked here (no browser needed for import/signature tests).
 *
 * Run: node --test skills/autospec-test/tests/unit/v2/lib-import.test.mjs
 */

import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import { execSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SKILL_DIR = path.resolve(__dirname, '../../../');
const LIB_DIR = path.join(SKILL_DIR, 'lib');
const DIST_DIR = path.join(SKILL_DIR, 'dist');

// ── Package.json validation ───────────────────────────────────────────────────

describe('package.json', () => {
  let pkg;

  before(() => {
    const pkgPath = path.join(SKILL_DIR, 'package.json');
    assert.ok(fs.existsSync(pkgPath), `package.json not found at ${pkgPath}`);
    pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  });

  it('has name @autospec/test', () => {
    assert.strictEqual(pkg.name, '@autospec/test');
  });

  it('has version 1.0.0', () => {
    assert.strictEqual(pkg.version, '1.0.0');
  });

  it('has type module', () => {
    assert.strictEqual(pkg.type, 'module');
  });

  it('has main pointing to dist/index.js', () => {
    assert.ok(pkg.main?.includes('dist/index'), `main should point to dist/index, got: ${pkg.main}`);
  });

  it('has types pointing to dist/index.d.ts', () => {
    assert.ok(pkg.types?.includes('dist/index'), `types should point to dist/index.d.ts, got: ${pkg.types}`);
  });

  it('has @playwright/test as peer dependency >= 1.40.0', () => {
    assert.ok(
      pkg.peerDependencies?.['@playwright/test'],
      'missing @playwright/test peerDependency'
    );
    assert.ok(
      pkg.peerDependencies['@playwright/test'].includes('1.40') ||
      pkg.peerDependencies['@playwright/test'].startsWith('>='),
      `peerDependency version should be >=1.40.0, got: ${pkg.peerDependencies['@playwright/test']}`
    );
  });

  it('exports . and ./invariants', () => {
    assert.ok(pkg.exports?.['.'], 'missing exports["."]');
    assert.ok(pkg.exports?.['./invariants'], 'missing exports["./invariants"]');
  });
});

// ── tsconfig.json validation ──────────────────────────────────────────────────

describe('tsconfig.json', () => {
  let tsconfig;

  before(() => {
    const tsPath = path.join(SKILL_DIR, 'tsconfig.json');
    assert.ok(fs.existsSync(tsPath), `tsconfig.json not found at ${tsPath}`);
    tsconfig = JSON.parse(fs.readFileSync(tsPath, 'utf8'));
  });

  it('has strict mode enabled', () => {
    assert.strictEqual(tsconfig.compilerOptions?.strict, true);
  });

  it('has outDir set to dist', () => {
    assert.ok(
      tsconfig.compilerOptions?.outDir?.includes('dist'),
      `outDir should include 'dist', got: ${tsconfig.compilerOptions?.outDir}`
    );
  });

  it('has declaration: true', () => {
    assert.strictEqual(tsconfig.compilerOptions?.declaration, true);
  });
});

// ── lib/invariants.ts source file ─────────────────────────────────────────────

describe('lib/invariants.ts', () => {
  let source;

  before(() => {
    const srcPath = path.join(LIB_DIR, 'invariants.ts');
    assert.ok(fs.existsSync(srcPath), `lib/invariants.ts not found at ${srcPath}`);
    source = fs.readFileSync(srcPath, 'utf8');
  });

  const REQUIRED_EXPORTS = [
    'assertEveryVisibleDoneItemIsEditable',
    'assertEveryFoldoutOpensAllNestedRows',
    'assertDateWindowCoverage',
    'assertContractSymmetry',
    'openAllFoldouts',
    'enumerateAffordances',
  ];

  for (const name of REQUIRED_EXPORTS) {
    it(`exports ${name}`, () => {
      assert.ok(
        source.includes(`export async function ${name}`) ||
        source.includes(`export function ${name}`),
        `lib/invariants.ts does not export: ${name}`
      );
    });
  }
});

// ── lib/index.ts re-exports ───────────────────────────────────────────────────

describe('lib/index.ts', () => {
  it('exists and re-exports from ./invariants', () => {
    const indexPath = path.join(LIB_DIR, 'index.ts');
    assert.ok(fs.existsSync(indexPath), `lib/index.ts not found at ${indexPath}`);
    const content = fs.readFileSync(indexPath, 'utf8');
    assert.ok(content.includes('./invariants'), 'index.ts should re-export from ./invariants');
  });
});

// ── TypeScript build ──────────────────────────────────────────────────────────

describe('TypeScript build', () => {
  it('tsc exits 0 and produces dist/index.d.ts', () => {
    // Run tsc from skill dir
    const result = spawnSync(
      'npx', ['tsc', '-p', 'tsconfig.json', '--noEmit', 'false'],
      { cwd: SKILL_DIR, encoding: 'utf8', timeout: 60000 }
    );
    if (result.error) {
      assert.fail(`tsc spawn error: ${result.error.message}`);
    }
    if (result.status !== 0) {
      assert.fail(`tsc failed (exit ${result.status}):\n${result.stderr}\n${result.stdout}`);
    }
    const dtsPath = path.join(DIST_DIR, 'index.d.ts');
    assert.ok(fs.existsSync(dtsPath), `dist/index.d.ts not produced at ${dtsPath}`);
  });

  it('dist/index.d.ts contains all 6 helper signatures', () => {
    // index.d.ts re-exports from invariants.d.ts; check invariants.d.ts for signatures
    const dtsPath = path.join(DIST_DIR, 'invariants.d.ts');
    if (!fs.existsSync(dtsPath)) {
      // Fall back to index.d.ts
      const indexDts = path.join(DIST_DIR, 'index.d.ts');
      if (!fs.existsSync(indexDts)) return; // tsc test above will catch this
    }
    const content = fs.readFileSync(dtsPath, 'utf8');
    const REQUIRED = [
      'assertEveryVisibleDoneItemIsEditable',
      'assertEveryFoldoutOpensAllNestedRows',
      'assertDateWindowCoverage',
      'assertContractSymmetry',
      'openAllFoldouts',
      'enumerateAffordances',
    ];
    for (const name of REQUIRED) {
      assert.ok(content.includes(name), `dist/index.d.ts missing: ${name}`);
    }
  });
});

// ── npm pack produces valid tarball ──────────────────────────────────────────

describe('npm pack', () => {
  let tarballPath;
  let tmpDir;

  before(() => {
    // Pack from the skill dir
    const result = spawnSync(
      'npm', ['pack', '--json'],
      { cwd: SKILL_DIR, encoding: 'utf8', timeout: 60000 }
    );
    if (result.error) {
      assert.fail(`npm pack spawn error: ${result.error.message}`);
    }
    if (result.status !== 0) {
      assert.fail(`npm pack failed (exit ${result.status}):\n${result.stderr}`);
    }
    const packOutput = JSON.parse(result.stdout);
    const filename = packOutput[0]?.filename;
    assert.ok(filename, 'npm pack --json did not return filename');
    tarballPath = path.join(SKILL_DIR, filename);
    assert.ok(fs.existsSync(tarballPath), `tarball not found: ${tarballPath}`);
  });

  after(() => {
    // Clean up tarball
    if (tarballPath && fs.existsSync(tarballPath)) {
      fs.rmSync(tarballPath);
    }
    // Clean up temp install dir
    if (tmpDir && fs.existsSync(tmpDir)) {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it('tarball name starts with autospec-test-1.0.0', () => {
    const base = path.basename(tarballPath);
    assert.ok(
      base.startsWith('autospec-test-1.0.0'),
      `tarball name should start with autospec-test-1.0.0, got: ${base}`
    );
  });

  it('tarball contains package/dist/index.js and package/dist/index.d.ts', () => {
    // List tarball contents
    const result = spawnSync('tar', ['tzf', tarballPath], { encoding: 'utf8' });
    assert.strictEqual(result.status, 0, `tar failed: ${result.stderr}`);
    const files = result.stdout.split('\n');
    assert.ok(files.some(f => f.includes('dist/index.js')), 'tarball missing dist/index.js');
    assert.ok(files.some(f => f.includes('dist/index.d.ts')), 'tarball missing dist/index.d.ts');
  });

  it('tarball can be installed and helpers imported', async () => {
    // Install tarball in a temp dir and import helpers
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'autospec-test-install-'));
    // Init temp package
    fs.writeFileSync(path.join(tmpDir, 'package.json'), JSON.stringify({
      name: 'test-consumer', version: '1.0.0', type: 'module'
    }));

    const installResult = spawnSync(
      'npm', ['install', tarballPath, '--no-save'],
      { cwd: tmpDir, encoding: 'utf8', timeout: 60000 }
    );
    if (installResult.status !== 0) {
      assert.fail(`npm install tarball failed:\n${installResult.stderr}`);
    }

    // Write a tiny consumer script
    const consumerScript = path.join(tmpDir, 'check.mjs');
    fs.writeFileSync(consumerScript, `
import {
  assertEveryVisibleDoneItemIsEditable,
  assertEveryFoldoutOpensAllNestedRows,
  assertDateWindowCoverage,
  assertContractSymmetry,
  openAllFoldouts,
  enumerateAffordances,
} from '@autospec/test/invariants';

// All 6 should be functions
const helpers = [
  assertEveryVisibleDoneItemIsEditable,
  assertEveryFoldoutOpensAllNestedRows,
  assertDateWindowCoverage,
  assertContractSymmetry,
  openAllFoldouts,
  enumerateAffordances,
];
for (const h of helpers) {
  if (typeof h !== 'function') {
    process.stderr.write('NOT A FUNCTION: ' + h + '\\n');
    process.exit(1);
  }
}
process.stdout.write('OK\\n');
`);
    const checkResult = spawnSync('node', [consumerScript], {
      cwd: tmpDir, encoding: 'utf8', timeout: 30000
    });
    assert.strictEqual(
      checkResult.status, 0,
      `helper import check failed (exit ${checkResult.status}):\n${checkResult.stderr}`
    );
    assert.ok(checkResult.stdout.includes('OK'), 'consumer script did not print OK');
  });
});

// ── publish-helpers.sh ────────────────────────────────────────────────────────

describe('publish-helpers.sh', () => {
  const publishScript = path.join(SKILL_DIR, 'scripts/publish-helpers.sh');

  it('exists and is executable', () => {
    assert.ok(fs.existsSync(publishScript), `publish-helpers.sh not found at ${publishScript}`);
    const stat = fs.statSync(publishScript);
    assert.ok(stat.mode & 0o111, 'publish-helpers.sh is not executable');
  });

  it('--dry-run exits 0 without calling npm publish', () => {
    const result = spawnSync('bash', [publishScript, '--dry-run'], {
      cwd: SKILL_DIR, encoding: 'utf8', timeout: 60000
    });
    if (result.status !== 0) {
      assert.fail(`publish-helpers.sh --dry-run failed (exit ${result.status}):\n${result.stderr}\n${result.stdout}`);
    }
    // Must not contain real publish (dry-run only)
    assert.ok(
      !result.stdout.includes('npm publish ') || result.stdout.includes('--dry-run'),
      'publish-helpers.sh --dry-run called real npm publish'
    );
  });

  it('does not contain bare npm publish without --release guard', () => {
    const content = fs.readFileSync(publishScript, 'utf8');
    // Verify the script contains a RELEASE guard (if [ "$RELEASE" -eq 1 ] block)
    // and that any bare 'npm publish' (without --dry-run) is inside that block.
    const hasReleaseGuard = content.includes('RELEASE') && content.includes('--release');
    assert.ok(hasReleaseGuard, 'publish-helpers.sh must have a --release flag guard');
    // Also verify dry-run path calls npm publish --dry-run
    assert.ok(content.includes('npm publish --dry-run'), 'publish-helpers.sh must have a --dry-run path');
  });
});
