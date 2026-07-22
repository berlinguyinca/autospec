import { execFileSync } from 'node:child_process';

function available(command) {
  try { execFileSync('which', [command], { stdio: 'ignore' }); return true; } catch { return false; }
}

export function countMatchingRows(csvPath, predicate, tableName = 'data') {
  const sqlite = available('sqlite3');
  const python = available('python3');
  if (!sqlite && !python) {
    throw new Error('sqlite3 or python3 is required');
  }
  if (sqlite) {
    const sql = [
      '.mode csv',
      `.import "${csvPath.replace(/"/g, '\\"')}" ${tableName}`,
      `SELECT COUNT(*) FROM ${tableName} WHERE ${predicate};`,
    ].join('\n');
    return Number(execFileSync('sqlite3', [':memory:'], { input: sql, encoding: 'utf8' }).trim()) || 0;
  }
  const script = [
    'import csv, sqlite3, sys',
    'path, predicate, table = sys.argv[1:]',
    'with open(path, newline="") as source:',
    '    rows = list(csv.reader(source))',
    'headers, values = rows[0], rows[1:]',
    'db = sqlite3.connect(":memory:")',
    'columns = ", ".join(\'"\' + h.replace(\'"\', \'""\') + \'" TEXT\' for h in headers)',
    'db.execute("CREATE TABLE " + table + " (" + columns + ")")',
    'db.executemany("INSERT INTO " + table + " VALUES (" + ",".join("?" for _ in headers) + ")", values)',
    'print(db.execute("SELECT COUNT(*) FROM " + table + " WHERE " + predicate).fetchone()[0])',
  ].join('\n');
  return Number(execFileSync('python3', ['-c', script, csvPath, predicate, tableName], { encoding: 'utf8' }).trim()) || 0;
}
