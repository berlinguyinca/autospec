/**
 * server.mjs — Express dev server for target-contract-symmetry-bait.
 *
 * Serves the index.html SPA and an API endpoint that returns task events.
 * The bait: t-1 and t-2 have events, t-3 returns empty events array.
 */

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PORT = parseInt(process.env.PORT ?? '3003', 10);

const INDEX_HTML = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf8');

// API data: t-1 and t-2 have events, t-3 deliberately has none (the bait)
const EVENTS = {
  't-1': { events: [{ task_id: 't-1', date: '2026-05-14', editable: true }] },
  't-2': { events: [{ task_id: 't-2', date: '2026-05-14', editable: true }] },
  't-3': { events: [] },  // BAIT: UI shows t-3 but API returns nothing
};

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${PORT}`);

  if (url.pathname === '/api/household/timeline') {
    const taskId = url.searchParams.get('task_id') ??
                   url.searchParams.get('member_id');
    // Simple lookup by task_id param; falls back to empty
    let data = taskId && EVENTS[taskId] ? EVENTS[taskId] : { events: [] };

    // Also support from/to filtering (returns matching task data)
    const from = url.searchParams.get('from');
    if (from && !taskId) {
      // Return all events for the date range — t-3 still empty
      data = {
        events: [
          ...EVENTS['t-1'].events,
          ...EVENTS['t-2'].events,
          // t-3 intentionally absent
        ],
      };
    }

    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(data));
    return;
  }

  // Serve SPA for all other routes
  res.writeHead(200, { 'Content-Type': 'text/html' });
  res.end(INDEX_HTML);
});

const HOST = process.env.HOST ?? '127.0.0.1';

server.listen(PORT, HOST, () => {
  process.stdout.write(`[contract-symmetry-bait] listening on http://${HOST}:${PORT}\n`);
});

export { server };
