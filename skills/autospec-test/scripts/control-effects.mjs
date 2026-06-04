#!/usr/bin/env node
// scripts/control-effects.mjs
// Control inventory + effect-assertion taxonomy (implements spec §§controls/effects
// from docs/specs/2026-05-27-playwright-control-effect-coverage-design.md and
// docs/specs/2026-06-04-autospec-playwright-authoring-design.md §6 step 4).
//
// Exports:
//   CONTROL_KINDS        — ordered array of control-kind descriptors
//   EFFECT_TAXONOMY      — ordered array of effect-assertion descriptors
//   inventoryControls(page, opts) -> Promise<ControlEntry[]>
//   effectPromptFragment(inventory, opts) -> string
//
// A ControlEntry has the shape:
//   { kind, selector, label, enabled, defaultValue, readOnly }
//
// Export: CONTROL_KINDS, EFFECT_TAXONOMY, inventoryControls, effectPromptFragment
// CLI:    node control-effects.mjs --url <url> [--read-only-selectors <csv>]

import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

const __filename = fileURLToPath(import.meta.url);

// ── Control kinds (spec §2 "Control Inventory Contract") ──────────────────────

/**
 * Ordered list of UI control categories autospec must enumerate.
 * Each entry:
 *   kind        — canonical snake_case identifier
 *   label       — human-readable label used in reports / prompts
 *   selectors   — CSS/ARIA selectors used to find controls of this kind
 *   description — one-liner for the prompt template
 */
export const CONTROL_KINDS = [
    {
        kind: 'button',
        label: 'Button',
        selectors: ['button', '[role=button]'],
        description: 'Clickable button or element with role="button"',
    },
    {
        kind: 'link',
        label: 'Link / Tab',
        selectors: ['a[href]', '[role=tab]', '[role=link]'],
        description: 'Anchor links and ARIA tab/link elements',
    },
    {
        kind: 'text_input',
        label: 'Text / Search Input',
        selectors: [
            'input[type=text]',
            'input[type=search]',
            'input[type=email]',
            'input[type=password]',
            'input[type=url]',
            'input[type=tel]',
            'input[type=number]',
            'input:not([type])',
            'textarea',
        ],
        description: 'Free-text inputs and textareas',
    },
    {
        kind: 'checkbox',
        label: 'Checkbox',
        selectors: ['input[type=checkbox]', '[role=checkbox]'],
        description: 'Binary toggle checkbox or ARIA equivalent',
    },
    {
        kind: 'radio',
        label: 'Radio',
        selectors: ['input[type=radio]', '[role=radio]'],
        description: 'Single-select radio button',
    },
    {
        kind: 'switch',
        label: 'Switch',
        selectors: ['[role=switch]'],
        description: 'On/off toggle switch (ARIA role="switch")',
    },
    {
        kind: 'native_select',
        label: 'Native Select',
        selectors: ['select'],
        description: 'HTML native <select> dropdown',
    },
    {
        kind: 'custom_dropdown',
        label: 'Custom Dropdown / Listbox / Combobox',
        selectors: ['[role=listbox]', '[role=combobox]', '[aria-haspopup=listbox]', '[aria-haspopup=true]'],
        description: 'Custom ARIA-based dropdown, listbox, or combobox',
    },
    {
        kind: 'date_input',
        label: 'Date / Time Input',
        selectors: [
            'input[type=date]',
            'input[type=time]',
            'input[type=datetime-local]',
            'input[type=month]',
            'input[type=week]',
        ],
        description: 'Date, time, or datetime picker input',
    },
    {
        kind: 'disclosure',
        label: 'Disclosure / Foldout',
        selectors: ['details', '[aria-expanded]', '[role=button][aria-controls]'],
        description: 'Expandable/collapsible disclosure section',
    },
    {
        kind: 'file_upload',
        label: 'File Upload',
        selectors: ['input[type=file]'],
        description: 'File upload input',
    },
    {
        kind: 'drag_drop',
        label: 'Drag / Drop',
        selectors: ['[draggable=true]', '[role=listitem][draggable]'],
        description: 'Draggable element for drag-and-drop interactions',
    },
];

// ── Effect taxonomy (spec §3 "Effect Assertions") ─────────────────────────────

/**
 * Ordered list of measurable effect categories that a non-read-only control
 * interaction MUST assert at least one of.
 * Each entry:
 *   effect      — canonical snake_case identifier
 *   label       — human-readable label for reports
 *   description — one-liner for the prompt template
 *   example     — example assertion snippet
 */
export const EFFECT_TAXONOMY = [
    {
        effect: 'ui_state_change',
        label: 'Visible UI State Change',
        description: 'The rendered DOM visibly changes after interaction (element appears/disappears/changes text).',
        example: 'await expect(page.getByTestId("status-badge")).toHaveText("Active");',
    },
    {
        effect: 'network_request_change',
        label: 'Outbound Request Change',
        description: 'An outbound REST/fetch request URL, query, headers, or JSON body changes as expected.',
        example: 'const req = await page.waitForRequest(r => r.url().includes("/api/items")); assert(req.postDataJSON().taxonomy === "compound");',
    },
    {
        effect: 'persistence_change',
        label: 'Persisted State Change',
        description: 'A setting, record, or local/session-storage value changes and survives a page reload.',
        example: 'await page.reload(); await expect(page.getByTestId("selected-taxonomy")).toHaveText("compound");',
    },
    {
        effect: 'list_or_table_change',
        label: 'List / Table / Chart Change',
        description: 'A resulting list, table, or chart changes in a way tied to the selected value.',
        example: 'await expect(page.getByRole("row")).toHaveCount(3);',
    },
    {
        effect: 'navigation',
        label: 'Navigation',
        description: 'The browser navigates to a new URL or route after interaction.',
        example: 'await expect(page).toHaveURL(/\\/dashboard/);',
    },
    {
        effect: 'api_state_via_helper',
        label: 'API State via Real-API Helper',
        description: 'After a UI write (create/update/delete), a real-API helper confirms the backend reflects the change.',
        example: 'const items = await apiHelper.getItems(); assert(items.some(i => i.name === "New Item"));',
    },
];

// ── inventoryControls ─────────────────────────────────────────────────────────

/**
 * Inventory all visible, enabled, and disabled controls on the current page.
 * Requires an active Playwright `page` object.
 *
 * @param {import('playwright').Page} page - active Playwright page
 * @param {object} [opts]
 * @param {string[]} [opts.readOnlySelectors=[]] - CSS selectors declared read-only in test.yml
 * @param {string[]} [opts.kinds] - subset of CONTROL_KINDS.kind to inventory (default: all)
 * @returns {Promise<ControlEntry[]>}
 *
 * @typedef {object} ControlEntry
 * @property {string} kind - from CONTROL_KINDS[].kind
 * @property {string} selector - CSS/ARIA selector used to locate the element
 * @property {string} label - accessible name or best-effort label
 * @property {boolean} enabled - whether the control is interactive (not disabled)
 * @property {string|null} defaultValue - current value for inputs/selects; null otherwise
 * @property {boolean} readOnly - declared read-only in test.yml opts.readOnlySelectors
 */
export async function inventoryControls(page, opts = {}) {
    const { readOnlySelectors = [], kinds } = opts;

    const kindFilter = kinds && kinds.length > 0
        ? new Set(kinds)
        : null;

    const activeKinds = kindFilter
        ? CONTROL_KINDS.filter(k => kindFilter.has(k.kind))
        : CONTROL_KINDS;

    const results = [];

    for (const kindDef of activeKinds) {
        for (const sel of kindDef.selectors) {
            let elements;
            try {
                elements = await page.locator(sel).all();
            } catch {
                continue;
            }

            for (const el of elements) {
                try {
                    const visible = await el.isVisible();
                    if (!visible) continue;

                    const disabled = await el.isDisabled();
                    const label = await _bestLabel(el, page);
                    const defaultValue = await _defaultValue(el, kindDef.kind);

                    // Check if element matches any read-only selector
                    const roMatch = readOnlySelectors.length > 0
                        ? await _matchesAnySelector(el, page, readOnlySelectors)
                        : false;

                    results.push({
                        kind: kindDef.kind,
                        selector: sel,
                        label,
                        enabled: !disabled,
                        defaultValue,
                        readOnly: roMatch,
                    });
                } catch {
                    // Element may have gone stale during enumeration — skip
                }
            }
        }
    }

    return results;
}

/**
 * Try to derive a human-readable accessible name for an element.
 * Falls back through: aria-label → aria-labelledby → associated <label> → placeholder → text content.
 *
 * @param {import('playwright').Locator} el
 * @param {import('playwright').Page} page
 * @returns {Promise<string>}
 */
async function _bestLabel(el, page) {
    try {
        const ariaLabel = await el.getAttribute('aria-label');
        if (ariaLabel && ariaLabel.trim()) return ariaLabel.trim();

        const labelledBy = await el.getAttribute('aria-labelledby');
        if (labelledBy) {
            try {
                const labelEl = page.locator(`#${CSS.escape(labelledBy)}`);
                const labelText = await labelEl.textContent({ timeout: 500 });
                if (labelText && labelText.trim()) return labelText.trim();
            } catch { /* ignore */ }
        }

        const id = await el.getAttribute('id');
        if (id) {
            try {
                const labelEl = page.locator(`label[for="${id}"]`);
                const labelText = await labelEl.textContent({ timeout: 500 });
                if (labelText && labelText.trim()) return labelText.trim();
            } catch { /* ignore */ }
        }

        const placeholder = await el.getAttribute('placeholder');
        if (placeholder && placeholder.trim()) return placeholder.trim();

        const text = await el.textContent({ timeout: 500 });
        if (text && text.trim()) return text.trim().slice(0, 80);
    } catch { /* ignore */ }

    return '';
}

/**
 * Get the current default value for value-bearing controls.
 *
 * @param {import('playwright').Locator} el
 * @param {string} kind
 * @returns {Promise<string|null>}
 */
async function _defaultValue(el, kind) {
    try {
        if (kind === 'native_select' || kind === 'text_input' || kind === 'date_input') {
            return await el.inputValue({ timeout: 500 });
        }
        if (kind === 'checkbox' || kind === 'radio') {
            const checked = await el.isChecked({ timeout: 500 });
            return checked ? 'checked' : 'unchecked';
        }
    } catch { /* ignore */ }
    return null;
}

/**
 * Check whether an element matches any of the given CSS selectors.
 *
 * @param {import('playwright').Locator} el
 * @param {import('playwright').Page} _page
 * @param {string[]} selectors
 * @returns {Promise<boolean>}
 */
async function _matchesAnySelector(el, _page, selectors) {
    for (const sel of selectors) {
        try {
            const matches = await el.evaluate(
                (node, s) => node.matches(s),
                sel
            );
            if (matches) return true;
        } catch { /* ignore */ }
    }
    return false;
}

// ── effectPromptFragment ──────────────────────────────────────────────────────

/**
 * Generate the effect-assertion section of the Playwright authoring prompt.
 * Embeds the control inventory and effect taxonomy into a structured prompt
 * fragment that the author subagent receives.
 *
 * @param {ControlEntry[]} inventory - output of inventoryControls()
 * @param {object} [opts]
 * @param {string} [opts.routeCluster=''] - name of the route cluster being authored
 * @param {string[]} [opts.readOnlySelectors=[]] - selectors declared read-only
 * @returns {string} - prompt fragment (markdown)
 */
export function effectPromptFragment(inventory, opts = {}) {
    const { routeCluster = '', readOnlySelectors = [] } = opts;

    const lines = [];

    lines.push('## Control inventory + effect-assertion requirements');
    lines.push('');

    if (routeCluster) {
        lines.push(`Route cluster: **${routeCluster}**`);
        lines.push('');
    }

    lines.push(
        'For every visible form or navigation control on each page in this cluster:'
    );
    lines.push('');

    // Control kinds table
    lines.push('### Control categories to enumerate');
    lines.push('');
    lines.push('| Kind | Selectors | Description |');
    lines.push('|---|---|---|');
    for (const ck of CONTROL_KINDS) {
        lines.push(`| ${ck.label} | \`${ck.selectors.join('`, `')}\` | ${ck.description} |`);
    }
    lines.push('');

    // Effect taxonomy
    lines.push('### Required effect assertions (each non-read-only control must assert ≥1)');
    lines.push('');
    lines.push('| Effect | Description | Example |');
    lines.push('|---|---|---|');
    for (const et of EFFECT_TAXONOMY) {
        lines.push(`| **${et.label}** | ${et.description} | \`${et.example}\` |`);
    }
    lines.push('');

    // Specific inventory
    if (inventory && inventory.length > 0) {
        lines.push('### Controls found on this cluster (pre-inventoried)');
        lines.push('');

        const byKind = {};
        for (const entry of inventory) {
            if (!byKind[entry.kind]) byKind[entry.kind] = [];
            byKind[entry.kind].push(entry);
        }

        for (const [kind, entries] of Object.entries(byKind)) {
            const kindDef = CONTROL_KINDS.find(k => k.kind === kind);
            const kindLabel = kindDef ? kindDef.label : kind;
            lines.push(`**${kindLabel}** (${entries.length}):`);
            for (const entry of entries) {
                const readOnlyNote = entry.readOnly ? ' *(declared read-only)*' : '';
                const enabledNote = entry.enabled ? '' : ' *(disabled)*';
                const valueNote = entry.defaultValue !== null ? ` default: \`${entry.defaultValue}\`` : '';
                const labelPart = entry.label ? ` "${entry.label}"` : '';
                lines.push(`  - selector: \`${entry.selector}\`${labelPart}${valueNote}${enabledNote}${readOnlyNote}`);
            }
            lines.push('');
        }
    }

    // Hard rules
    lines.push('### Hard rules for authored specs');
    lines.push('');
    lines.push('- **Every enabled non-read-only control** must have a test that exercises it and asserts ≥1 effect from the table above.');
    lines.push('- **Native `<select>`**: must assert it has ≥2 options (unless documented read-only); select a non-default option; submit/trigger; assert the selected value reaches the request or output.');
    lines.push('- **Custom dropdown/listbox/combobox**: open it, assert options are visible, choose a non-default option, assert request/output/persistence effect.');
    lines.push('- **Disabled visible control** = FAIL unless `.autospec/test.yml` declares it read-only with a reason.');
    lines.push('- **Decorative control** (no measurable effect after interaction) = `product_bug` violation — never suppress the assertion.');
    lines.push('- All API-state checks must go through the real-API helper, not direct DB writes.');

    if (readOnlySelectors.length > 0) {
        lines.push('');
        lines.push('### Selectors declared read-only (skip effect assertion for these)');
        for (const sel of readOnlySelectors) {
            lines.push(`  - \`${sel}\``);
        }
    }

    lines.push('');
    return lines.join('\n');
}

// ── CLI entrypoint ────────────────────────────────────────────────────────────

if (process.argv[1] && fs.existsSync(process.argv[1]) &&
    fs.realpathSync(path.resolve(process.argv[1])) === fs.realpathSync(path.resolve(__filename))) {
    const args = process.argv.slice(2);
    const getArg = (flag) => {
        const idx = args.indexOf(flag);
        return idx >= 0 ? args[idx + 1] : null;
    };

    const url = getArg('--url');
    const readOnlyCsv = getArg('--read-only-selectors');

    if (!url) {
        process.stderr.write('Usage: control-effects.mjs --url <url> [--read-only-selectors <csv>]\n');
        process.stderr.write('\nWithout --url, prints taxonomy tables:\n');
        // Print taxonomy tables as JSON
        process.stdout.write(JSON.stringify({ CONTROL_KINDS, EFFECT_TAXONOMY }, null, 2) + '\n');
        process.exit(0);
    }

    const readOnlySelectors = readOnlyCsv ? readOnlyCsv.split(',').map(s => s.trim()).filter(Boolean) : [];

    try {
        // Dynamic import of playwright — only needed for CLI inventory mode
        const { chromium } = await import('playwright');
        const browser = await chromium.launch({ headless: true });
        const page = await browser.newPage();
        await page.goto(url, { waitUntil: 'networkidle', timeout: 30000 });

        const inventory = await inventoryControls(page, { readOnlySelectors });
        await browser.close();

        process.stdout.write(JSON.stringify(inventory, null, 2) + '\n');
    } catch (err) {
        process.stderr.write(`control-effects: ${err.message}\n`);
        process.exit(2);
    }
}
