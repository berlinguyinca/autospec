// Copy the v1 JSON Schemas from the repository root into this package so
// `npm pack` ships them: the schemas live at ../../schemas (outside the
// package directory), so a `files` entry cannot reach them directly.
//
// Zero dependencies (node:fs / node:path). Run automatically by `npm pack`
// via the `prepack` script; also run it manually before a dry-run pack to
// inspect the tarball contents.

import { cpSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const sourceDir = join(here, '..', '..', 'schemas');
const targetDir = join(here, 'schemas');

const names = readdirSync(sourceDir).filter(
  (name) => name.startsWith('autospec-protocol-') && name.endsWith('.v1.schema.json'),
);

if (names.length === 0) {
  console.error(
    `prepack: no autospec-protocol-*.v1.schema.json files found in ${sourceDir}`,
  );
  process.exit(1);
}

// Start from a clean copy so removed schemas do not linger in the tarball.
rmSync(targetDir, { recursive: true, force: true });
mkdirSync(targetDir, { recursive: true });
for (const name of names) {
  cpSync(join(sourceDir, name), join(targetDir, name));
}

console.log(`prepack: copied ${names.length} schema(s) into ${targetDir}`);
