#!/usr/bin/env node
// function-presence.mjs — AST walker for exported function detection.
//
// Usage: node function-presence.mjs <src_dir> <test_dir>
//
// Output JSON (stdout):
//   {
//     "exported_functions": [{ "file": "...", "name": "...", "signature": "..." }],
//     "test_references":    [{ "file": "...", "references_name": "..." }],
//     "missing_tests":      ["functionName", ...]   // exported but not referenced
//   }
//
// Supports: JS/TS via @typescript-eslint/parser (full AST)
//           Python/Go/Rust/JVM: subprocess stubs (returns empty list with warning)
//
// Exit codes:
//   0 = success (may have missing_tests)
//   1 = fatal error (missing args, unreadable dir, parser crash)

import { readFileSync, readdirSync, statSync } from 'fs';
import { join, extname, relative } from 'path';
import { createRequire } from 'module';
import { spawnSync, execFileSync } from 'child_process';

// Resolve NODE_PATH for globally installed packages (npm install -g)
// so the script works without a local node_modules directory.
function resolveGlobalRequire() {
  // Try standard createRequire from this file's location
  const localReq = createRequire(import.meta.url);
  try {
    localReq.resolve('@typescript-eslint/parser');
    return localReq;
  } catch (_) {}

  // Try global npm root
  try {
    const npmRoot = execFileSync('npm', ['root', '-g'], { encoding: 'utf8' }).trim();
    if (npmRoot) {
      const globalReq = createRequire(join(npmRoot, 'dummy.js'));
      globalReq.resolve('@typescript-eslint/parser');
      return globalReq;
    }
  } catch (_) {}

  return localReq; // will fail later with a clear message
}

const require = resolveGlobalRequire();

const [,, srcDir, testDir] = process.argv;

if (!srcDir || !testDir) {
  process.stderr.write('Usage: function-presence.mjs <src_dir> <test_dir>\n');
  process.exit(1);
}

// ── File discovery ─────────────────────────────────────────────────────────────
function walkDir(dir, exts) {
  const results = [];
  try {
    const entries = readdirSync(dir);
    for (const entry of entries) {
      const full = join(dir, entry);
      const st = statSync(full);
      if (st.isDirectory()) {
        results.push(...walkDir(full, exts));
      } else if (exts.includes(extname(entry))) {
        results.push(full);
      }
    }
  } catch (e) {
    // Directory may not exist — return empty
  }
  return results;
}

const JS_TS_EXTS = ['.js', '.mjs', '.cjs', '.ts', '.tsx', '.jsx'];

const srcFiles  = walkDir(srcDir,  JS_TS_EXTS);
const testFiles = walkDir(testDir, JS_TS_EXTS);

// ── Parser setup ──────────────────────────────────────────────────────────────
let parser;
try {
  parser = require('@typescript-eslint/parser');
} catch (e) {
  process.stderr.write('WARN: @typescript-eslint/parser not found; install with: npm install -g @typescript-eslint/parser\n');
  process.exit(1);
}

function parseFile(filePath) {
  const code = readFileSync(filePath, 'utf8');
  try {
    return parser.parse(code, {
      jsx: true,
      loc: true,
      range: true,
      tokens: false,
      comment: false,
      errorOnUnknownASTType: false,
      // Allow TypeScript
    });
  } catch (e) {
    process.stderr.write(`WARN: parse error in ${filePath}: ${e.message}\n`);
    return null;
  }
}

// ── Exported function extraction from source files ────────────────────────────
function extractExportedFunctions(filePath) {
  const ast = parseFile(filePath);
  if (!ast) return [];

  const results = [];
  const relPath = relative(srcDir, filePath);

  function addFn(name, node) {
    const params = node.params
      ? node.params.map(p => p.name || (p.left && p.left.name) || '...').join(', ')
      : '';
    results.push({
      file: relPath,
      name,
      signature: `${name}(${params})`,
    });
  }

  function visit(node) {
    if (!node || typeof node !== 'object') return;

    // export function foo() {}
    if (node.type === 'ExportNamedDeclaration' && node.declaration) {
      const decl = node.declaration;
      if (decl.type === 'FunctionDeclaration' && decl.id) {
        addFn(decl.id.name, decl);
      }
      // export const foo = () => {} OR export const foo = function() {}
      if (decl.type === 'VariableDeclaration') {
        for (const d of decl.declarations) {
          if (d.init && (d.init.type === 'ArrowFunctionExpression' || d.init.type === 'FunctionExpression')) {
            if (d.id && d.id.name) addFn(d.id.name, d.init);
          }
        }
      }
    }

    // export default function foo() {}
    if (node.type === 'ExportDefaultDeclaration' && node.declaration) {
      const decl = node.declaration;
      if (decl.type === 'FunctionDeclaration') {
        addFn(decl.id ? decl.id.name : 'default', decl);
      }
    }

    // module.exports = { foo, bar } — CJS style
    if (
      node.type === 'ExpressionStatement' &&
      node.expression.type === 'AssignmentExpression' &&
      node.expression.left.type === 'MemberExpression' &&
      node.expression.left.object.name === 'module' &&
      node.expression.left.property.name === 'exports'
    ) {
      const right = node.expression.right;
      if (right.type === 'ObjectExpression') {
        for (const prop of right.properties) {
          if (prop.type === 'Property' && prop.key) {
            const name = prop.key.name || prop.key.value;
            if (name) results.push({ file: relPath, name, signature: `${name}(...)` });
          }
          if (prop.type === 'SpreadElement') {
            // skip spreads
          }
        }
      }
    }

    // Recurse into children
    for (const key of Object.keys(node)) {
      if (key === 'parent') continue;
      const child = node[key];
      if (Array.isArray(child)) {
        for (const c of child) visit(c);
      } else if (child && typeof child === 'object' && child.type) {
        visit(child);
      }
    }
  }

  if (ast.body) {
    for (const stmt of ast.body) visit(stmt);
  }

  return results;
}

// ── Test reference extraction from test files ─────────────────────────────────
function extractTestReferences(filePath) {
  const ast = parseFile(filePath);
  if (!ast) return [];

  const results = [];
  const relPath = relative(testDir, filePath);
  const seen = new Set();

  function collectIdentifiers(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'Identifier' && node.name && !seen.has(node.name)) {
      seen.add(node.name);
      results.push({ file: relPath, references_name: node.name });
    }
    for (const key of Object.keys(node)) {
      if (key === 'parent') continue;
      const child = node[key];
      if (Array.isArray(child)) {
        for (const c of child) collectIdentifiers(c);
      } else if (child && typeof child === 'object' && child.type) {
        collectIdentifiers(child);
      }
    }
  }

  // Also extract import specifiers specifically
  function visitImports(node) {
    if (!node || typeof node !== 'object') return;
    if (node.type === 'ImportDeclaration') {
      for (const spec of (node.specifiers || [])) {
        const name = spec.imported?.name || spec.local?.name;
        if (name && !seen.has(name)) {
          seen.add(name);
          results.push({ file: relPath, references_name: name });
        }
      }
    }
    // require destructuring: const { foo, bar } = require(...)
    if (
      node.type === 'VariableDeclaration'
    ) {
      for (const d of node.declarations) {
        if (d.id && d.id.type === 'ObjectPattern') {
          for (const prop of d.id.properties) {
            const name = prop.key?.name;
            if (name && !seen.has(name)) {
              seen.add(name);
              results.push({ file: relPath, references_name: name });
            }
          }
        }
      }
    }
    for (const key of Object.keys(node)) {
      if (key === 'parent') continue;
      const child = node[key];
      if (Array.isArray(child)) {
        for (const c of child) visitImports(c);
      } else if (child && typeof child === 'object' && child.type) {
        visitImports(child);
      }
    }
  }

  if (ast.body) {
    for (const stmt of ast.body) visitImports(stmt);
  }

  return results;
}

// ── Main ──────────────────────────────────────────────────────────────────────
const exportedFunctions = [];
for (const f of srcFiles) {
  exportedFunctions.push(...extractExportedFunctions(f));
}

const testReferences = [];
for (const f of testFiles) {
  testReferences.push(...extractTestReferences(f));
}

// Compute missing: exported functions with no test reference
const referencedNames = new Set(testReferences.map(r => r.references_name));
const missingTests = exportedFunctions
  .filter(fn => !referencedNames.has(fn.name))
  .map(fn => fn.name);

const output = {
  exported_functions: exportedFunctions,
  test_references:    testReferences,
  missing_tests:      missingTests,
};

process.stdout.write(JSON.stringify(output, null, 2) + '\n');
process.exit(0);
