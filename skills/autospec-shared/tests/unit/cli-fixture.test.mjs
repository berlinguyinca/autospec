import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const cliPath = path.resolve(__dirname, '../fixtures/reverse-engineer-bait/src/cli.mjs');

function runCli(args = []) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [cliPath, ...args]);
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', code => resolve({ code, stdout, stderr }));
  });
}

test('CLI fixture emits its greeting without debug logging', async () => {
  const source = await readFile(cliPath, 'utf8');
  assert.doesNotMatch(source, /console\.(log|debug|info|warn|error)\s*\(/);
  assert.doesNotMatch(source, /\bdebugger\b/);

  const result = await runCli(['Ada']);
  assert.equal(result.code, 0);
  assert.equal(result.stdout, 'Hello, Ada!\n');
  assert.equal(result.stderr, '');
});
