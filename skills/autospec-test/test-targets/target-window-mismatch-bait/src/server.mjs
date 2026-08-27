/**
 * server.mjs — Express dev server for target-window-mismatch-bait.
 *
 * Serves the index.html SPA and an API endpoint that always returns data
 * for a 3-day window regardless of the `from`/`to` parameters.
 *
 * The mismatch: the UI widget declares data-window-days="7" but the client
 * JS fetches from=today-3d. This server just responds to whatever query arrives.
 */

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PORT = parseInt(process.env.PORT ?? '3002', 10);

const INDEX_HTML = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf8');

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (url.pathname === '/api/household/timeline') {
    // Return streak data — doesn't validate window, accepts any from/to
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      streak_days: 3,
      from: url.searchParams.get('from') ?? '',
      to: url.searchParams.get('to') ?? '',
      events: [],
    }));
    return;
  }

  // Serve SPA for all other routes
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end(INDEX_HTML);
});

const HOST = process.env.HOST ?? '127.0.0.1';

server.listen(PORT, HOST, () => {
  process.stdout.write(`[window-mismatch-bait] listening on http://${HOST}:${PORT}\n`);
});

export { server };
