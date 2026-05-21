// target-mode-ii-fixture: Minimal Express+SQLite app for Mode II scope-violation test.
// One test intentionally mutates an out-of-scope row to trigger scope-violation + restore.

export interface Family {
  id: string;
  name: string;
}

export function getFamilyById(db: Family[], id: string): Family | undefined {
  return db.find(f => f.id === id);
}

export function updateFamily(db: Family[], id: string, name: string): boolean {
  const family = db.find(f => f.id === id);
  if (!family) return false;
  family.name = name;
  return true;
}
