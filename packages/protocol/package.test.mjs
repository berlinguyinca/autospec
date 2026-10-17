// @autospec/protocol packaging contract.
//
// The generated TypeScript declarations (types.d.ts) and the v1 JSON Schemas
// must be publishable as one package: `npm pack` from packages/protocol must
// ship both, and the manifest must identify the package so a consumer
// (pi-engineering) can import it by path or tarball.
//
// The schemas live at the repository root (../../schemas), outside this
// package directory, so prepack.mjs copies the autospec-protocol-*.v1
// schemas into packages/protocol/schemas/ before packing (wired as the
// `prepack` script; the copies are git-ignored).
//
// Run with `node --test package.test.mjs` (the CI job protocol-node runs this
// together with fixtures.test.mjs).

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(readFileSync(join(here, 'package.json'), 'utf8'));

test('the manifest identifies @autospec/protocol 1.0.0', () => {
  assert.equal(pkg.name, '@autospec/protocol');
  assert.equal(pkg.version, '1.0.0');
});

test('the tarball ships the schemas and the generated types', () => {
  // The tarball is the unit a consumer installs; assert on its contents rather
  // than on the manifest alone, so a `files` entry that silently stops
  // matching still fails here.
  //
  // Run prepack first (the `prepack` script copies the v1 schemas from
  // ../../schemas into this package), then dry-run pack and inspect the
  // file list from the JSON output — not a manifest string.
  execFileSync('npm', ['run', 'prepack', '--silent'], { cwd: here, encoding: 'utf8' });
  const packed = execFileSync(
    'npm',
    ['pack', '--dry-run', '--json'],
    { cwd: here, encoding: 'utf8' },
  );
  // `npm run` prints the script banner before the pack JSON; start at the
  // first `[` so the prepack banner does not break the parse.
  const manifest = JSON.parse(packed.slice(packed.indexOf('[')))[0];
  const paths = manifest.files.map((file) => file.path);

  assert.ok(
    paths.includes('package.json'),
    `the tarball must ship the manifest; got: ${paths.join(', ')}`,
  );
  assert.ok(
    paths.includes('types.d.ts'),
    `the tarball must ship the generated TypeScript declarations; got: ${paths.join(', ')}`,
  );
  const shippedSchemas = paths.filter((path) =>
    path.startsWith('schemas/autospec-protocol-') &&
    path.endsWith('.v1.schema.json'),
  );
  assert.ok(
    shippedSchemas.length > 0,
    `the tarball must ship at least one v1 schema, e.g. ` +
      `schemas/autospec-protocol-envelope.v1.schema.json; got: ${paths.join(', ')}`,
  );
  assert.ok(
    paths.includes('schemas/autospec-protocol-envelope.v1.schema.json'),
    `the tarball must ship schemas/autospec-protocol-envelope.v1.schema.json; ` +
      `got: ${paths.join(', ')}`,
  );
});

test('a real pack (in a temp dir) ships the schemas and the types', () => {
  // The dry-run listing is what `npm pack` would produce; a real pack into a
  // temporary directory proves the tarball a consumer actually installs
  // contains the schemas, not just the manifest's claim.
  const dir = mkdtempSync(join(tmpdir(), 'autospec-protocol-'));
  try {
    execFileSync('npm', ['run', 'prepack', '--silent'], { cwd: here, encoding: 'utf8' });
    execFileSync(
      'npm',
      ['pack', `--pack-destination=${dir}`, '--silent'],
      { cwd: here, encoding: 'utf8' },
    );
    const tgz = execFileSync('ls', [dir], {
      encoding: 'utf8',
    })
      .trim()
      .split('\n')
      .find((name) => name.endsWith('.tgz'));
    assert.ok(tgz, `a tarball must have been packed into ${dir}`);
    const listing = execFileSync('tar', ['-tzf', join(dir, tgz)], {
      encoding: 'utf8',
    })
      .split('\n')
      .filter((line) => line.length > 0);
    assert.ok(
      listing.includes('package/package.json'),
      `the tarball must ship the manifest; got: ${listing.join(', ')}`,
    );
    assert.ok(
      listing.includes('package/types.d.ts'),
      `the tarball must ship the generated TypeScript declarations; got: ${listing.join(', ')}`,
    );
    assert.ok(
      listing.includes(
        'package/schemas/autospec-protocol-envelope.v1.schema.json',
      ),
      `the tarball must ship the v1 schemas; got: ${listing.join(', ')}`,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a consumer can import the package by path', async () => {
  // `import ... from '<path>/package.json' with { type: 'json' }` is the
  // zero-dependency import a consumer resolves without installing; the `types`
  // field is what a TypeScript consumer picks up for the declarations.
  const imported = await import(new URL('./package.json', import.meta.url), {
    with: { type: 'json' },
  });
  assert.equal(imported.default.name, '@autospec/protocol');
  assert.equal(pkg.types, 'types.d.ts');
  assert.equal(pkg.exports['.'].types, './types.d.ts');
});
