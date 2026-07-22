#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

function usage() { console.error('Usage: gen-ui-audit-doc.mjs --input audit.json [--output docs.md]'); }
function args(argv) {
  const out = {}; for (let i = 2; i < argv.length; i += 1) {
    if (argv[i] === '--input') out.input = argv[++i];
    else if (argv[i] === '--output') out.output = argv[++i];
    else if (argv[i] === '--help') out.help = true;
  } return out;
}
function readRoutes(value) {
  if (Array.isArray(value)) return value;
  if (Array.isArray(value?.routes)) return value.routes;
  return [];
}
function val(route, ...keys) { for (const key of keys) if (route?.[key] !== undefined) return route[key]; return null; }
export function renderAuditDoc(audit) {
  const routes = readRoutes(audit);
  const lines = ['# UI screenshot audit', '', `Generated: ${audit.generatedAt ?? audit.generated_at ?? 'unknown'}`, ''];
  for (const route of routes) {
    const name = typeof route === 'string' ? route : val(route, 'route', 'path') ?? 'unknown';
    const screenshots = typeof route === 'object' ? val(route, 'screenshots', 'screenshotLinks') : null;
    const links = screenshots && typeof screenshots === 'object' ? Object.entries(screenshots).map(([k, v]) => `[${k}](${v})`).join(', ') : (Array.isArray(screenshots) ? screenshots.map(v => `[screenshot](${v})`).join(', ') : 'none');
    const counts = typeof route === 'object' ? val(route, 'primitiveCounts', 'primitive_counts', 'primitives') : null;
    const findings = typeof route === 'object' ? val(route, 'consoleFindings', 'console_findings', 'console') : null;
    const status = typeof route === 'object' ? val(route, 'status', 'pageStatus', 'page_status') ?? 'unknown' : 'unknown';
    const authShell = typeof route === 'object' ? Boolean(val(route, 'authShell', 'auth_shell')) : false;
    const overflow = typeof route === 'object' ? Boolean(val(route, 'overflow', 'documentOverflow', 'document_overflow')) : false;
    lines.push(`## \`${name}\``, '', `- Page status: **${status}**`, `- Auth shell detected: **${authShell}**`, `- Document overflow: **${overflow}**`, `- Screenshots: ${links}`);
    if (counts && typeof counts === 'object') lines.push(`- Primitive counts: ${Object.entries(counts).map(([k,v]) => `\`${k}\`=${v}`).join(', ')}`);
    if (Array.isArray(findings) && findings.length) lines.push(`- Console findings: ${findings.map(v => `\`${String(v)}\``).join(', ')}`);
    lines.push('', 'Generated test guidance: verify visible content, auth-shell absence, and document-level overflow at desktop, tablet, and mobile viewports.', '');
  }
  return `${lines.join('\n')}\n`;
}
if (import.meta.url === `file://${process.argv[1]}`) {
  const options = args(process.argv);
  if (options.help) usage();
  else {
    if (!options.input) throw new Error('input audit artifact is required');
    const doc = renderAuditDoc(JSON.parse(fs.readFileSync(options.input, 'utf8')));
    if (options.output) { fs.mkdirSync(path.dirname(options.output), { recursive: true }); fs.writeFileSync(options.output, doc); } else process.stdout.write(doc);
  }
}
