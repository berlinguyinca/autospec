// skills/autospec-test/tests/unit/control-effects.test.mjs
// node --test  (Node.js built-in test runner)
// Tests for scripts/control-effects.mjs
// Covers: CONTROL_KINDS, EFFECT_TAXONOMY, inventoryControls (mock page), effectPromptFragment

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPTS_DIR = path.resolve(__dirname, '../../scripts');

const { CONTROL_KINDS, EFFECT_TAXONOMY, inventoryControls, effectPromptFragment } =
    await import(`file://${SCRIPTS_DIR}/control-effects.mjs`);

// ── CONTROL_KINDS ─────────────────────────────────────────────────────────────

test('CONTROL_KINDS: is a non-empty array', () => {
    assert.ok(Array.isArray(CONTROL_KINDS), 'CONTROL_KINDS must be an array');
    assert.ok(CONTROL_KINDS.length > 0, 'CONTROL_KINDS must not be empty');
});

test('CONTROL_KINDS: each entry has required fields', () => {
    for (const ck of CONTROL_KINDS) {
        assert.ok(typeof ck.kind === 'string' && ck.kind.length > 0,
            `kind missing or empty in ${JSON.stringify(ck)}`);
        assert.ok(typeof ck.label === 'string' && ck.label.length > 0,
            `label missing or empty in ${JSON.stringify(ck)}`);
        assert.ok(Array.isArray(ck.selectors) && ck.selectors.length > 0,
            `selectors missing or empty in ${JSON.stringify(ck)}`);
        assert.ok(typeof ck.description === 'string' && ck.description.length > 0,
            `description missing or empty in ${JSON.stringify(ck)}`);
    }
});

test('CONTROL_KINDS: includes button, native_select, checkbox, text_input, custom_dropdown', () => {
    const kinds = CONTROL_KINDS.map(k => k.kind);
    for (const required of ['button', 'native_select', 'checkbox', 'text_input', 'custom_dropdown']) {
        assert.ok(kinds.includes(required), `Missing kind: ${required}`);
    }
});

test('CONTROL_KINDS: kinds are unique', () => {
    const kinds = CONTROL_KINDS.map(k => k.kind);
    const unique = new Set(kinds);
    assert.equal(unique.size, kinds.length, 'CONTROL_KINDS has duplicate kind values');
});

test('CONTROL_KINDS: native_select kind uses "select" selector', () => {
    const ns = CONTROL_KINDS.find(k => k.kind === 'native_select');
    assert.ok(ns, 'native_select kind must exist');
    assert.ok(ns.selectors.includes('select'), 'native_select must include "select" selector');
});

test('CONTROL_KINDS: custom_dropdown kind includes ARIA role selectors', () => {
    const cd = CONTROL_KINDS.find(k => k.kind === 'custom_dropdown');
    assert.ok(cd, 'custom_dropdown kind must exist');
    const hasListbox = cd.selectors.some(s => s.includes('listbox'));
    const hasCombobox = cd.selectors.some(s => s.includes('combobox'));
    assert.ok(hasListbox || hasCombobox,
        'custom_dropdown must include [role=listbox] or [role=combobox] selector');
});

// ── EFFECT_TAXONOMY ───────────────────────────────────────────────────────────

test('EFFECT_TAXONOMY: is a non-empty array', () => {
    assert.ok(Array.isArray(EFFECT_TAXONOMY), 'EFFECT_TAXONOMY must be an array');
    assert.ok(EFFECT_TAXONOMY.length > 0, 'EFFECT_TAXONOMY must not be empty');
});

test('EFFECT_TAXONOMY: each entry has required fields', () => {
    for (const et of EFFECT_TAXONOMY) {
        assert.ok(typeof et.effect === 'string' && et.effect.length > 0,
            `effect field missing in ${JSON.stringify(et)}`);
        assert.ok(typeof et.label === 'string' && et.label.length > 0,
            `label field missing in ${JSON.stringify(et)}`);
        assert.ok(typeof et.description === 'string' && et.description.length > 0,
            `description field missing in ${JSON.stringify(et)}`);
        assert.ok(typeof et.example === 'string' && et.example.length > 0,
            `example field missing in ${JSON.stringify(et)}`);
    }
});

test('EFFECT_TAXONOMY: includes all required effect classes', () => {
    const effects = EFFECT_TAXONOMY.map(e => e.effect);
    for (const required of [
        'ui_state_change',
        'network_request_change',
        'persistence_change',
        'api_state_via_helper',
    ]) {
        assert.ok(effects.includes(required), `Missing effect: ${required}`);
    }
});

test('EFFECT_TAXONOMY: effects are unique', () => {
    const effects = EFFECT_TAXONOMY.map(e => e.effect);
    const unique = new Set(effects);
    assert.equal(unique.size, effects.length, 'EFFECT_TAXONOMY has duplicate effect values');
});

// ── inventoryControls (mock page) ─────────────────────────────────────────────

/**
 * Build a minimal mock Playwright page object that returns pre-canned locators.
 * Each entry in `controlMap` has shape:
 *   { selector, visible, disabled, ariaLabel, value, checked, matchesReadOnly }
 */
function makeMockPage(elements) {
    // Group elements by selector
    const bySelector = {};
    for (const el of elements) {
        if (!bySelector[el.selector]) bySelector[el.selector] = [];
        bySelector[el.selector].push(el);
    }

    function makeLocator(el) {
        return {
            isVisible: async () => el.visible !== false,
            isDisabled: async () => el.disabled === true,
            isChecked: async () => el.checked === true,
            inputValue: async () => el.value || '',
            getAttribute: async (attr) => {
                if (attr === 'aria-label') return el.ariaLabel || null;
                if (attr === 'aria-labelledby') return null;
                if (attr === 'id') return el.id || null;
                if (attr === 'placeholder') return el.placeholder || null;
                return null;
            },
            textContent: async () => el.textContent || '',
            evaluate: async (fn, s) => {
                // For _matchesAnySelector: simulate el.matches(s)
                return el.matchesReadOnly === true && s === el.readOnlySel;
            },
            all: async () => [],
        };
    }

    return {
        locator: (sel) => {
            const elList = bySelector[sel] || [];
            return {
                all: async () => elList.map(el => makeLocator(el)),
                textContent: async () => '',
            };
        },
    };
}

test('inventoryControls: returns empty array for empty page', async () => {
    const page = makeMockPage([]);
    const result = await inventoryControls(page, {});
    assert.deepEqual(result, []);
});

test('inventoryControls: includes visible button', async () => {
    const page = makeMockPage([
        { selector: 'button', visible: true, disabled: false, textContent: 'Save' },
    ]);
    const result = await inventoryControls(page, {});
    const btn = result.find(e => e.kind === 'button');
    assert.ok(btn, 'Expected a button entry');
    assert.equal(btn.enabled, true);
    assert.equal(btn.readOnly, false);
});

test('inventoryControls: excludes invisible elements', async () => {
    const page = makeMockPage([
        { selector: 'button', visible: false, disabled: false, textContent: 'Hidden' },
    ]);
    const result = await inventoryControls(page, {});
    assert.equal(result.length, 0, 'Hidden elements must not appear in inventory');
});

test('inventoryControls: marks disabled control as enabled=false', async () => {
    const page = makeMockPage([
        { selector: 'button', visible: true, disabled: true, textContent: 'Disabled Btn' },
    ]);
    const result = await inventoryControls(page, {});
    const btn = result.find(e => e.kind === 'button');
    assert.ok(btn, 'Expected a button entry');
    assert.equal(btn.enabled, false, 'disabled button must have enabled=false');
});

test('inventoryControls: captures aria-label as label', async () => {
    const page = makeMockPage([
        { selector: 'button', visible: true, disabled: false, ariaLabel: 'Submit form' },
    ]);
    const result = await inventoryControls(page, {});
    const btn = result.find(e => e.kind === 'button');
    assert.ok(btn, 'Expected a button entry');
    assert.equal(btn.label, 'Submit form');
});

test('inventoryControls: captures native select value', async () => {
    const page = makeMockPage([
        { selector: 'select', visible: true, disabled: false, value: 'lipid' },
    ]);
    const result = await inventoryControls(page, {});
    const sel = result.find(e => e.kind === 'native_select');
    assert.ok(sel, 'Expected a native_select entry');
    assert.equal(sel.defaultValue, 'lipid');
});

test('inventoryControls: checkbox reports checked state', async () => {
    const page = makeMockPage([
        { selector: 'input[type=checkbox]', visible: true, disabled: false, checked: true },
    ]);
    const result = await inventoryControls(page, {});
    const cb = result.find(e => e.kind === 'checkbox');
    assert.ok(cb, 'Expected a checkbox entry');
    assert.equal(cb.defaultValue, 'checked');
});

test('inventoryControls: filters to requested kinds only', async () => {
    const page = makeMockPage([
        { selector: 'button', visible: true, disabled: false, textContent: 'Go' },
        { selector: 'select', visible: true, disabled: false, value: 'a' },
    ]);
    const result = await inventoryControls(page, { kinds: ['native_select'] });
    assert.ok(!result.some(e => e.kind === 'button'), 'button must be excluded when not in kinds');
    assert.ok(result.some(e => e.kind === 'native_select'), 'native_select must appear');
});

// ── effectPromptFragment ──────────────────────────────────────────────────────

test('effectPromptFragment: returns non-empty string', () => {
    const fragment = effectPromptFragment([], {});
    assert.ok(typeof fragment === 'string' && fragment.length > 0,
        'effectPromptFragment must return a non-empty string');
});

test('effectPromptFragment: contains control kinds section', () => {
    const fragment = effectPromptFragment([], {});
    assert.ok(fragment.includes('Control categories to enumerate'),
        'Must include control categories header');
    assert.ok(fragment.includes('Native Select'), 'Must mention Native Select kind');
    assert.ok(fragment.includes('Button'), 'Must mention Button kind');
});

test('effectPromptFragment: contains effect taxonomy section', () => {
    const fragment = effectPromptFragment([], {});
    assert.ok(fragment.includes('Required effect assertions'),
        'Must include effect assertions header');
    assert.ok(fragment.includes('Visible UI State Change'),
        'Must mention UI state change effect');
    assert.ok(fragment.includes('Outbound Request Change'),
        'Must mention network request change effect');
});

test('effectPromptFragment: includes hard rules for authored specs', () => {
    const fragment = effectPromptFragment([], {});
    assert.ok(fragment.includes('Hard rules for authored specs'),
        'Must include hard rules section');
    assert.ok(fragment.includes('Decorative control'),
        'Must mention decorative-control violation rule');
    assert.ok(fragment.includes('real-API helper'),
        'Must mention real-API helper requirement');
});

test('effectPromptFragment: includes route cluster when provided', () => {
    const fragment = effectPromptFragment([], { routeCluster: '/dashboard' });
    assert.ok(fragment.includes('/dashboard'), 'Must include route cluster in output');
});

test('effectPromptFragment: lists pre-inventoried controls by kind', () => {
    const inventory = [
        { kind: 'button', selector: 'button', label: 'Save', enabled: true, defaultValue: null, readOnly: false },
        { kind: 'native_select', selector: 'select', label: 'Taxonomy', enabled: true, defaultValue: 'lipid', readOnly: false },
    ];
    const fragment = effectPromptFragment(inventory, {});
    assert.ok(fragment.includes('Controls found on this cluster'), 'Must include inventory section');
    assert.ok(fragment.includes('Save'), 'Must mention button label');
    assert.ok(fragment.includes('Taxonomy'), 'Must mention select label');
    assert.ok(fragment.includes('lipid'), 'Must mention default value');
});

test('effectPromptFragment: marks read-only controls in inventory', () => {
    const inventory = [
        { kind: 'button', selector: 'button[disabled]', label: 'Archive', enabled: false, defaultValue: null, readOnly: true },
    ];
    const fragment = effectPromptFragment(inventory, {});
    assert.ok(fragment.includes('declared read-only'), 'Must mark read-only controls');
});

test('effectPromptFragment: includes read-only selectors section when provided', () => {
    const fragment = effectPromptFragment([], { readOnlySelectors: ['select[disabled]', '.read-only-ctrl'] });
    assert.ok(fragment.includes('Selectors declared read-only'), 'Must include read-only selectors section');
    assert.ok(fragment.includes('select[disabled]'), 'Must list read-only selector');
});

test('effectPromptFragment: native select rule explicitly mentions 2 options', () => {
    const fragment = effectPromptFragment([], {});
    assert.ok(fragment.includes('≥2 options') || fragment.includes('at least 2') || fragment.includes('two options'),
        'Must mention that native select must have 2+ options');
});
