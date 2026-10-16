// @autospec/protocol packaging contract.
//
// The generated TypeScript declarations (types.d.ts) and the v1 JSON Schemas
// must be publishable as one package: `npm pack` from packages/protocol must
// ship both, and the manifest must identify the package so a consumer
// (pi-engineering) can import it by path or tarball.
//
// Run with `node --test package.test.mjs` (the CI job protocol-node runs this
// together with fixtures.test.mjs).

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync } from 'node:fs';
import { test } from 'node:test';

const pkg = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
);

test('the manifest identifies @autospec/protocol 1.0.0', () => {
  assert.equal(pkg.name, '@autospec/protocol');
  assert.equal(pkg.version, '1.0.0');
});

test('the tarball ships the schemas and the generated types', () => {
  // The tarball is the unit a consumer installs; assert on its contents rather
  // than on the manifest alone, so a `files` entry that silently stops
  // matching still fails here.
  const dir = new URL('.', import.meta.url).pathname;
  mkdirSync(`${dir}node_modules/.cache`, { recursive: true });
  const tgz = execFileSync(
    'npm',
    ['pack', `--pack-destination=${dir}node_modules/.cache`, '--silent'],
    { cwd: dir, encoding: 'utf8' },
  )
    .trim()
    .split('\n')
    .at(-1);
  const listing = execFileSync('tar', ['-tzf', tgz], {
    cwd: `${dir}node_modules/.cache`,
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
  // The schemas live at the repository root (schemas/), not inside this
  // package, so the manifest must point at them rather than copy them: the
  // package's contract is the generated types, rendered from those schemas by
  // autospec_core::protocol::codegen.
  assert.equal(pkg.schemas, '../../schemas');
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
