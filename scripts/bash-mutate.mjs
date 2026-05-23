#!/usr/bin/env node
// scripts/bash-mutate.mjs — bash mutation engine for autospec mutation testing (M2).
//
// Applies 3 mutation operators to a bash source file and emits one mutant
// per matchable line into an output directory.
//
// Usage:
//   node bash-mutate.mjs --file <source.sh> --emit <output-dir>
//
// Operators:
//   OP_FLIP_EQ     - Flip == to != and != to == in test expressions
//   OP_DROP_ASSERT - Remove one assert/grep/run line (replace with : no-op)
//   OP_SWAP_LITERAL - Swap a string literal "X" with "" (empty string)
//
// Output (stdout): JSON array of mutant descriptors:
//   [{ id, operator, file, line, original, mutant, out_file }]
//
// Each mutant is written as a full copy of the source with one line changed.
// Exit 0 on success, 1 on error.

import fs from 'node:fs';
import path from 'node:path';

// ── CLI parsing ───────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
let sourceFile = '';
let emitDir = '';

for (let i = 0; i < args.length; i++) {
  if (args[i] === '--file' && args[i + 1]) { sourceFile = args[++i]; }
  else if (args[i] === '--emit' && args[i + 1]) { emitDir = args[++i]; }
  else if (args[i] === '--help') {
    process.stdout.write(
      'Usage: node bash-mutate.mjs --file <source.sh> --emit <output-dir>\n'
    );
    process.exit(0);
  }
}

if (!sourceFile || !emitDir) {
  process.stderr.write('bash-mutate.mjs: --file and --emit are required\n');
  process.exit(1);
}

if (!fs.existsSync(sourceFile)) {
  process.stderr.write(`bash-mutate.mjs: file not found: ${sourceFile}\n`);
  process.exit(1);
}

fs.mkdirSync(emitDir, { recursive: true });

// ── Operator implementations ──────────────────────────────────────────────────

// OP_FLIP_EQ: flip == to != and != to == in bash test/comparison lines.
// Targets: [ "$x" == "y" ], [[ $x == $y ]], if [ ... == ... ].
// Strategy: replace first == or != in lines containing [ or [[ or -eq/-ne.
function applyFlipEq(line) {
  // Only act on lines with test brackets or conditional operators
  if (!/\[/.test(line) && !/\bif\b/.test(line)) return null;
  // Flip != → == first (more specific), then == → !=
  if (/!=/.test(line)) {
    return line.replace(/!=/, '==');
  }
  // Avoid matching shebangs (#!/) or other == not in comparisons
  if (/(?<![!<>])={2}(?!=)/.test(line)) {
    return line.replace(/(?<![!<>])={2}(?!=)/, '!=');
  }
  return null;
}

// OP_DROP_ASSERT: replace an assert/grep/run assertion line with a no-op.
// Targets: lines starting with assert, run, grep -q, [ "$status", check.
const ASSERT_PATTERN = /^\s*(assert\b|run\s+|grep\s+-q|grep\s+-v|\[\s*"\$status|\[\s*\$status|check\s+)/;
function applyDropAssert(line) {
  if (!ASSERT_PATTERN.test(line)) return null;
  // Preserve leading whitespace, replace with bare colon (no-op)
  const indent = line.match(/^(\s*)/)[1];
  return `${indent}: # mutation:drop-assert`;
}

// OP_SWAP_LITERAL: swap the first non-empty double-quoted string literal with "".
// Targets: lines with "something" where something is non-empty.
const LITERAL_PATTERN = /"([^"\\]{1,40})"/;
function applySwapLiteral(line) {
  // Skip shebang, comments, variable assignments with empty strings
  if (/^\s*#/.test(line)) return null;
  if (/^#!/.test(line)) return null;
  const m = line.match(LITERAL_PATTERN);
  if (!m || m[1] === '') return null;
  // Replace only the first occurrence
  return line.replace(LITERAL_PATTERN, '""');
}

// ── Mutation engine ───────────────────────────────────────────────────────────

const OPERATORS = [
  { name: 'OP_FLIP_EQ', apply: applyFlipEq },
  { name: 'OP_DROP_ASSERT', apply: applyDropAssert },
  { name: 'OP_SWAP_LITERAL', apply: applySwapLiteral },
];

const content = fs.readFileSync(sourceFile, 'utf8');
const lines = content.split('\n');
const base = path.basename(sourceFile, path.extname(sourceFile));
const mutants = [];
let mutantId = 0;

for (const op of OPERATORS) {
  for (let i = 0; i < lines.length; i++) {
    const original = lines[i];
    const mutated = op.apply(original);
    if (mutated === null || mutated === original) continue;

    mutantId++;
    const outFile = path.join(emitDir, `${base}.mut${mutantId}.${op.name}.sh`);

    // Write mutant: copy of source with line i replaced
    const mutantLines = [...lines];
    mutantLines[i] = mutated;
    fs.writeFileSync(outFile, mutantLines.join('\n'), 'utf8');

    mutants.push({
      id: mutantId,
      operator: op.name,
      file: sourceFile,
      line: i + 1,
      original,
      mutant: mutated,
      out_file: outFile,
    });
  }
}

process.stdout.write(JSON.stringify(mutants, null, 2) + '\n');
process.exit(0);
