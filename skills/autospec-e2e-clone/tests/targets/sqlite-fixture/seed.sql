-- Seed data for sqlite-fixture integration target
CREATE TABLE IF NOT EXISTS items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    owner_email TEXT,
    value REAL,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER REFERENCES items(id),
    label TEXT NOT NULL
);

INSERT INTO items (name, owner_email, value) VALUES
    ('Widget A', 'user1@company.com', 9.99),
    ('Widget B', 'user2@company.com', 19.99),
    ('Widget C', 'user3@company.com', 4.99);

INSERT INTO tags (item_id, label) VALUES
    (1, 'featured'),
    (2, 'sale'),
    (3, 'new');
