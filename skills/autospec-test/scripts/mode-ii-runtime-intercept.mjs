// mode-ii-runtime-intercept.mjs — Mode II runtime scope-token enforcement.
//
// Exported API (for unit testing and Playwright global setup):
//   checkRequest({ method, url, scopeTokens, autospecDir }) → { allowed, violation, reason }
//
// Playwright integration:
//   import { installInterceptor } from './mode-ii-runtime-intercept.mjs';
//   installInterceptor(page, scopeTokens, autospecDir);
//
// Spec §5b Layer 2: every mutating request (POST/PUT/PATCH/DELETE) must match
// an allowed_path_patterns rule AND include the expected scope identifier.
// Violation → abort request + write .autospec/.scope-violation sentinel.

import fs from 'node:fs';
import path from 'node:path';

const MUTATING_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

/**
 * Check whether a request is allowed under the active scope tokens.
 *
 * @param {object} opts
 * @param {string} opts.method - HTTP method (uppercase)
 * @param {string} opts.url - Request URL or path
 * @param {Array}  opts.scopeTokens - Array of scope token objects from contract
 * @param {string} [opts.autospecDir] - Path to .autospec dir for sentinel writes
 * @returns {{ allowed: boolean, violation: boolean, reason: string }}
 */
export function checkRequest({ method, url, scopeTokens = [], autospecDir = null }) {
    const upperMethod = (method || '').toUpperCase();

    // Read-only methods are always allowed
    if (!MUTATING_METHODS.has(upperMethod)) {
        return { allowed: true, violation: false, reason: 'read_only_method' };
    }

    // No scope tokens → block all mutations (fail-closed)
    if (!scopeTokens || scopeTokens.length === 0) {
        const result = { allowed: false, violation: true, reason: 'no_scope_tokens_defined' };
        writeSentinel(autospecDir, result.reason, method, url);
        return result;
    }

    // Check each scope token; if any route_filter token matches, allow
    for (const token of scopeTokens) {
        if (token.kind === 'route_filter') {
            const methods = token.methods || MUTATING_METHODS;
            const methodSet = new Set(Array.isArray(methods) ? methods : [methods]);

            if (!methodSet.has(upperMethod)) {
                // This token doesn't govern this HTTP method — skip
                continue;
            }

            const patterns = token.allowed_path_patterns || [];
            for (const pattern of patterns) {
                try {
                    const re = new RegExp(pattern);
                    if (re.test(url)) {
                        return { allowed: true, violation: false, reason: 'scope_token_match' };
                    }
                } catch {
                    // Invalid regex in contract — skip this pattern
                }
            }

            // This route_filter token covers this method but URL didn't match
            const result = { allowed: false, violation: true, reason: `route_filter_violation: ${url} not in allowed_path_patterns` };
            writeSentinel(autospecDir, result.reason, method, url);
            return result;
        }
        // row_filter and method_allowlist tokens are enforced by postcheck, not intercept
    }

    // No route_filter token covered this request → allow (no route_filter = no URL restriction)
    return { allowed: true, violation: false, reason: 'no_route_filter_applies' };
}

/**
 * Write .autospec/.scope-violation sentinel file.
 */
function writeSentinel(autospecDir, reason, method, url) {
    if (!autospecDir) return;
    try {
        const sentinelPath = path.join(autospecDir, '.scope-violation');
        const content = JSON.stringify({
            ts: Math.floor(Date.now() / 1000),
            reason,
            method,
            url,
        }, null, 2) + '\n';
        fs.mkdirSync(autospecDir, { recursive: true });
        fs.writeFileSync(sentinelPath, content);
    } catch {
        // Non-fatal: sentinel write failure should not crash the test
    }
}

/**
 * Install the Mode II interceptor into a Playwright page/context.
 * Called from global-setup.ts or fixture.
 *
 * @param {object} context - Playwright BrowserContext
 * @param {Array} scopeTokens - from contract
 * @param {string} autospecDir - path to .autospec dir
 */
export async function installInterceptor(context, scopeTokens, autospecDir) {
    await context.route('**/*', (route) => {
        const request = route.request();
        const result = checkRequest({
            method: request.method(),
            url: request.url(),
            scopeTokens,
            autospecDir,
        });

        if (!result.allowed) {
            // Abort the request and write violation sentinel
            route.abort('blockedbyclient');
        } else {
            route.continue();
        }
    });
}

export default { checkRequest, installInterceptor };
