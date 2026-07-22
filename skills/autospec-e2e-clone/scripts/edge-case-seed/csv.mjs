import { readFileSync, writeFileSync, appendFileSync } from 'node:fs';

export function parseCsvLine(line) {
  const fields = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < line.length; i += 1) {
    const character = line[i];
    if (character === '"') {
      if (quoted && line[i + 1] === '"') {
        field += '"';
        i += 1;
      } else quoted = !quoted;
    } else if (character === ',' && !quoted) {
      fields.push(field);
      field = '';
    } else field += character;
  }
  fields.push(field);
  return fields;
}

export function toCsvLine(fields) {
  return fields.map((value) => {
    const field = String(value ?? '');
    return /[,"\n]/.test(field) ? `"${field.replaceAll('"', '""')}"` : field;
  }).join(',');
}

export function readCsv(filePath) {
  const lines = readFileSync(filePath, 'utf8').split(/\r?\n/).filter((line) => line.trim());
  if (lines.length === 0) return { headers: [], rows: [] };
  return { headers: parseCsvLine(lines[0]), rows: lines.slice(1).map(parseCsvLine) };
}

export function ensureSyntheticColumn(filePath) {
  const { headers, rows } = readCsv(filePath);
  if (headers.includes('_autospec_synthetic')) return headers;
  const updatedHeaders = [...headers, '_autospec_synthetic'];
  const output = [toCsvLine(updatedHeaders), ...rows.map((row) => toCsvLine([...row, '']))].join('\n') + '\n';
  writeFileSync(filePath, output, 'utf8');
  return updatedHeaders;
}

export function appendSyntheticRows(filePath, rows) {
  const headers = ensureSyntheticColumn(filePath);
  appendFileSync(filePath, rows.map((row) => toCsvLine(headers.map((header) => row[header] ?? ''))).join('\n') + '\n', 'utf8');
}
