#!/usr/bin/env node
// authoring-fixture — a tiny real HTTP server with 3 routes and embedded traps.
//
// Routes:
//   GET  /products          — list (heading has NO data-testid → trap1)
//   GET  /orders            — "Orders" heading text collides with nav link → trap2
//   GET  /account           — clean route
//   DELETE /api/products/:id — returns 200 but the row PERSISTS → trap3 (product bug)
//
// There is deliberately NO /api/test/reset endpoint — the Stage 2A reset-endpoint
// generator must synthesize a guard-env-gated one for this fixture.
//
// Real process, real HTTP, no DB (in-memory array). Boot via startServer().

import http from 'node:http';
import { productsPage, ordersPage, accountPage } from './src/views.mjs';

/** Seed rows (in-memory — re-seeded per startServer call). */
function seedRows() {
    return [
        { id: 1, name: 'Widget' },
        { id: 2, name: 'Gadget' },
        { id: 3, name: 'Gizmo' },
    ];
}

export function createServer() {
    let rows = seedRows();

    const server = http.createServer((req, res) => {
        const url = new URL(req.url, 'http://localhost');
        const pathname = url.pathname;

        // ── DELETE /api/products/:id — trap3: 200 OK but row is NOT removed ──
        const delMatch = pathname.match(/^\/api\/products\/(\d+)$/);
        if (req.method === 'DELETE' && delMatch) {
            // BUG (intentional): we acknowledge success but never mutate `rows`.
            // A correct implementation would: rows = rows.filter(r => r.id !== id);
            res.writeHead(200, { 'content-type': 'application/json' });
            res.end(JSON.stringify({ ok: true, deleted: Number(delMatch[1]) }));
            return;
        }

        // ── GET /api/products — JSON state (lets the test assert persistence) ──
        if (req.method === 'GET' && pathname === '/api/products') {
            res.writeHead(200, { 'content-type': 'application/json' });
            res.end(JSON.stringify({ rows }));
            return;
        }

        // ── HTML routes ──
        if (req.method === 'GET') {
            let html = null;
            if (pathname === '/products') html = productsPage(rows);
            else if (pathname === '/orders') html = ordersPage();
            else if (pathname === '/account') html = accountPage();
            if (html !== null) {
                res.writeHead(200, { 'content-type': 'text/html' });
                res.end(html);
                return;
            }
        }

        res.writeHead(404, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ error: 'Not found' }));
    });

    return server;
}

/** Start the fixture on an ephemeral port; resolves with {server, baseUrl, port}. */
export function startServer() {
    return new Promise((resolve) => {
        const server = createServer();
        server.listen(0, '127.0.0.1', () => {
            const { port } = server.address();
            resolve({ server, port, baseUrl: `http://127.0.0.1:${port}` });
        });
    });
}

/** The three crawlable HTML routes this fixture exposes. */
export const ROUTES = ['/products', '/orders', '/account'];

// CLI: boot on a fixed port for manual crawling (operator/full verification).
if (import.meta.url === `file://${process.argv[1]}`) {
    const port = Number(process.env.PORT) || 3000;
    const server = createServer();
    server.listen(port, () => {
        process.stdout.write(`authoring-fixture listening on http://localhost:${port}\n`);
    });
}
