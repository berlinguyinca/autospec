import assert from 'node:assert/strict';
import test from 'node:test';

import FIXTURES from './fixtures.mjs';

test('assertion-shift fixtures are immutable records with isolated defaults', () => {
    assert.ok(Object.isFrozen(FIXTURES));
    assert.ok(FIXTURES.every((record) => Object.isFrozen(record)));
    assert.ok(FIXTURES.every((record) => Object.isFrozen(record.expected)));
    assert.ok(FIXTURES.every((record) => Object.isFrozen(record.nonTestFilesChanged)));

    const original = FIXTURES[0].nonTestFilesChanged;
    assert.throws(() => original.push('src/changed.js'), TypeError);
    assert.throws(() => {
        FIXTURES[0].description = 'mutated';
    }, TypeError);
    assert.equal(FIXTURES[0].nonTestFilesChanged, original);
});

test('focused-marker fixtures retain their exact diff payloads', () => {
    const byId = new Map(FIXTURES.map((record) => [record.id, record.diff]));
    assert.match(byId.get('jest-03'), /\+test\.skip\('add'/);
    assert.match(byId.get('pytest-02'), /\+@pytest\.mark\.skip/);
    assert.match(byId.get('jest-10'), /-test\.skip\('add'/);
    assert.match(byId.get('pytest-05'), /-@pytest\.mark\.skip/);
});
