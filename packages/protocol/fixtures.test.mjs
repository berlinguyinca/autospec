// The shared golden fixtures (tests/fixtures/protocol/) asserted from the
// TypeScript toolchain. This file reads the same schemas and fixtures as
// crates/autospec-core/tests/protocol_fixtures.rs and must reach the same
// outcome for every record. It implements only the contract's schema subset
// (docs/contracts/autospec-protocol-v1.md) and, like the Rust loader, refuses
// any other keyword instead of ignoring it. No dependencies: node:test only.

import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SCHEMA_FILE = /^autospec-protocol-(.+)\.v1\.schema\.json$/;
const SUBSET = new Set([
  'type', 'properties', 'required', 'additionalProperties', 'enum', 'const',
  'items', '$ref', 'minimum', 'default', 'title', 'description', '$schema',
  '$id', '$defs',
]);
const ANNOTATIONS = new Set(['title', 'description', 'default']);
const SUPPORTED_VERSIONS = [1];
const UNDERSTOOD = ['x-known'];
const ENTITIES = [
  'mission', 'task', 'execution', 'worker', 'event', 'resource-record',
  'checkpoint', 'lease', 'budget',
];

const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);

// ---- schema subset -------------------------------------------------------

/** Reject, naming the keyword, anything outside the subset. Mirrors schema_ir.rs. */
function assertSubset(node, file, at, isRoot) {
  assert.ok(isObject(node), `${file}: ${at} must be a schema object`);
  for (const key of Object.keys(node)) {
    const rootOnly = key === '$schema' || key === '$id' || key === '$defs';
    if (!SUBSET.has(key) || (rootOnly && !isRoot)) {
      throw new Error(`UNSUPPORTED_SCHEMA_KEYWORD: \`${key}\` in ${file}`);
    }
  }
  if ('$ref' in node) {
    for (const key of Object.keys(node)) {
      assert.ok(key === '$ref' || ANNOTATIONS.has(key), `${file}: ${at}: \`${key}\` beside $ref`);
    }
    return;
  }
  assert.equal(typeof node.type, 'string', `${file}: ${at}: type must be one string`);
  assert.ok(
    ['object', 'array', 'string', 'integer', 'number', 'boolean'].includes(node.type),
    `${file}: ${at}: type ${node.type} is not in the subset`,
  );
  for (const [name, sub] of Object.entries(node.properties ?? {})) {
    assertSubset(sub, file, `${at}/properties/${name}`, false);
  }
  if (isObject(node.additionalProperties)) {
    assertSubset(node.additionalProperties, file, `${at}/additionalProperties`, false);
  }
  if (node.items !== undefined) assertSubset(node.items, file, `${at}/items`, false);
}

function loadSchemas(dir) {
  const schemas = new Map();
  for (const name of readdirSync(dir).sort()) {
    const match = SCHEMA_FILE.exec(name);
    if (!match) continue;
    const schema = JSON.parse(readFileSync(join(dir, name), 'utf8'));
    assertSubset(schema, name, '#', true);
    for (const [def, sub] of Object.entries(schema.$defs ?? {})) {
      assertSubset(sub, name, `#/$defs/${def}`, false);
    }
    schemas.set(match[1], schema);
  }
  return schemas;
}

const escapePointer = (key) => key.replaceAll('~', '~0').replaceAll('/', '~1');

/** Every violation as {pointer, rule}, like SchemaSet::validate. */
function validate(schemas, entity, value) {
  const schema = schemas.get(entity);
  if (!schema) return [{ pointer: '', rule: 'unknown-entity' }];
  const out = [];
  const check = (node, v, ptr) => {
    if (node.$ref !== undefined) {
      const name = /^#\/\$defs\/([^/]+)$/.exec(node.$ref)?.[1];
      const target = schema.$defs?.[name];
      assert.ok(target, `${entity}: unresolved $ref ${node.$ref}`);
      return check(target, v, ptr);
    }
    switch (node.type) {
      case 'object': {
        if (!isObject(v)) return out.push({ pointer: ptr, rule: 'type' });
        for (const key of node.required ?? []) {
          if (!(key in v)) out.push({ pointer: `${ptr}/${escapePointer(key)}`, rule: 'required' });
        }
        for (const [key, member] of Object.entries(v)) {
          const child = `${ptr}/${escapePointer(key)}`;
          if (node.properties && key in node.properties) check(node.properties[key], member, child);
          else if (isObject(node.additionalProperties)) check(node.additionalProperties, member, child);
          else if (node.additionalProperties === false) out.push({ pointer: child, rule: 'additionalProperties' });
        }
        return;
      }
      case 'array':
        if (!Array.isArray(v)) return out.push({ pointer: ptr, rule: 'type' });
        v.forEach((item, i) => check(node.items, item, `${ptr}/${i}`));
        return;
      case 'string':
        if (typeof v !== 'string') return out.push({ pointer: ptr, rule: 'type' });
        if (node.enum && !node.enum.includes(v)) out.push({ pointer: ptr, rule: 'enum' });
        if ('const' in node && node.const !== v) out.push({ pointer: ptr, rule: 'const' });
        return;
      case 'integer':
      case 'number':
        if (typeof v !== 'number' || !Number.isFinite(v) || (node.type === 'integer' && !Number.isInteger(v))) {
          return out.push({ pointer: ptr, rule: 'type' });
        }
        if (node.minimum !== undefined && v < node.minimum) out.push({ pointer: ptr, rule: 'minimum' });
        return;
      case 'boolean':
        if (typeof v !== 'boolean') out.push({ pointer: ptr, rule: 'type' });
        return;
      default:
        throw new Error(`unsupported type ${node.type}`);
    }
  };
  check(schema, value, '');
  return out;
}

// ---- envelope check and sequencer (contract rules, as in the Rust reader) ---

const SCHEMA_ID = /^autospec\.protocol\.[a-z0-9_-]+\.v[1-9][0-9]*$/;

/** The refusal class of a message, or the envelope fields when it is acceptable. */
function checkEnvelope(v) {
  const refuse = (code) => ({ error: code });
  if (!isObject(v)) return refuse('SCHEMA_VIOLATION');
  const text = (k) => typeof v[k] === 'string' && v[k] !== '';
  if (!text('schema') || !SCHEMA_ID.test(v.schema)) return refuse('SCHEMA_VIOLATION');
  if (!Number.isInteger(v.protocol_version) || v.protocol_version < 1) return refuse('SCHEMA_VIOLATION');
  if (!text('idempotency_key') || !text('mission_id')) return refuse('SCHEMA_VIOLATION');
  if (!Number.isInteger(v.sequence) || v.sequence < 1) return refuse('SCHEMA_VIOLATION');
  if (!Array.isArray(v.required_extensions)) return refuse('SCHEMA_VIOLATION');
  if ('extensions' in v && !isObject(v.extensions)) return refuse('SCHEMA_VIOLATION');
  for (const name of v.required_extensions) {
    if (typeof name !== 'string') return refuse('SCHEMA_VIOLATION');
    if (!UNDERSTOOD.includes(name)) return refuse('UNKNOWN_REQUIRED_FIELD');
    if (!(v.extensions && name in v.extensions)) return refuse('SCHEMA_VIOLATION');
  }
  if (!SUPPORTED_VERSIONS.includes(v.protocol_version)) return refuse('PROTOCOL_VERSION_UNSUPPORTED');
  return { envelope: v };
}

class MissionSequencer {
  #missions = new Map();

  accept(env, digest) {
    if (!this.#missions.has(env.mission_id)) this.#missions.set(env.mission_id, { mark: 0, seen: new Map() });
    const state = this.#missions.get(env.mission_id);
    const prior = state.seen.get(env.idempotency_key);
    if (prior) {
      if (prior.sequence !== env.sequence) return 'IDEMPOTENCY_KEY_REUSED';
      return prior.digest === digest ? 'duplicate' : 'IDEMPOTENCY_CONFLICT';
    }
    const expected = state.mark + 1;
    if (env.sequence > expected) return 'SEQUENCE_GAP';
    if (env.sequence < expected) return 'SEQUENCE_CONFLICT';
    state.seen.set(env.idempotency_key, { sequence: env.sequence, digest });
    state.mark = env.sequence;
    return 'applied';
  }
}

/** The same pipeline as outcome() in protocol_fixtures.rs. */
function outcome(schemas, sequencer, entity, record) {
  const checked = checkEnvelope(record);
  if (checked.error) return checked.error;
  if (validate(schemas, entity, record).length > 0) return 'violation';
  return sequencer.accept(checked.envelope, JSON.stringify(record));
}

// ---- tests -----------------------------------------------------------------

const schemas = loadSchemas(join(root, 'schemas'));
const fixture = (name) => JSON.parse(readFileSync(join(root, 'tests/fixtures/protocol', name), 'utf8'));
const valid = fixture('valid.json');
const invalid = fixture('invalid.json');

test('loads the ten protocol schemas and refuses a keyword outside the subset', () => {
  assert.equal(schemas.size, 10);
  assert.deepEqual([...schemas.keys()].sort(), [...ENTITIES, 'envelope'].sort());
  const bad = { type: 'object', properties: { schema: { type: 'string', pattern: '^a' } } };
  assert.throws(() => assertSubset(bad, 'x.json', '#', true), /UNSUPPORTED_SCHEMA_KEYWORD.*pattern/);
});

test('valid.json has one key per entity and every record is clean', () => {
  assert.deepEqual(Object.keys(valid).sort(), [...ENTITIES].sort());
  for (const entity of ENTITIES) {
    assert.ok(valid[entity].length >= 1, `${entity}: at least one record`);
    for (const record of valid[entity]) {
      assert.equal(record.schema, `autospec.protocol.${entity}.v1`);
      assert.deepEqual(validate(schemas, entity, record), [], entity);
      assert.equal(checkEnvelope(record).error, undefined, entity);
    }
  }
});

test('valid events are one mission with sequences 1 to 3 and all apply', () => {
  assert.deepEqual(valid.event.map((e) => e.sequence), [1, 2, 3]);
  assert.equal(new Set(valid.event.map((e) => e.mission_id)).size, 1);
  const sequencer = new MissionSequencer();
  for (const event of valid.event) assert.equal(outcome(schemas, sequencer, 'event', event), 'applied');
});

test('every invalid case reaches the same outcome as the Rust validator', () => {
  assert.ok(invalid.length >= 6);
  for (const c of invalid) {
    const sequencer = new MissionSequencer();
    for (const prior of c.after ?? []) {
      assert.equal(outcome(schemas, sequencer, c.entity, prior), 'applied', `${c.name}: prior record`);
    }
    assert.equal(outcome(schemas, sequencer, c.entity, c.record), c.expect, c.name);
  }
});

test('deleting any required field from a valid record is a violation', () => {
  for (const entity of ENTITIES) {
    const required = schemas.get(entity).required;
    for (const field of required) {
      const copy = structuredClone(valid[entity][0]);
      delete copy[field];
      const found = validate(schemas, entity, copy);
      assert.ok(
        found.some((v) => v.pointer === `/${field}` && v.rule === 'required'),
        `${entity}: deleting ${field} must be reported`,
      );
    }
  }
});

test('an unknown top-level field is a violation for every entity', () => {
  for (const entity of ENTITIES) {
    const copy = structuredClone(valid[entity][0]);
    copy.surprise = 1;
    assert.ok(
      validate(schemas, entity, copy).some((v) => v.pointer === '/surprise' && v.rule === 'additionalProperties'),
      entity,
    );
  }
});
