-- Seed data for mode-ii-real dogfood target
CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    email TEXT NOT NULL,
    name TEXT,
    phone TEXT,
    ssn TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS tasks (
    id SERIAL PRIMARY KEY,
    user_id INT REFERENCES users(id),
    title TEXT NOT NULL,
    status TEXT DEFAULT 'pending',
    created_at TIMESTAMPTZ DEFAULT NOW()
);

INSERT INTO users (email, name, phone, ssn) VALUES
    ('alice@corp.com',   'Alice Real',  '+49-30-12345', '000-11-2222'),
    ('bob@corp.com',     'Bob Real',    '+49-30-98765', '333-44-5555'),
    ('carol@corp.com',   'Carol Real',  '+49-30-55555', '666-77-8888');

INSERT INTO tasks (user_id, title, status) VALUES
    (1, 'task done today',          'done'),
    (2, 'task in progress',         'in_progress'),
    (3, 'task in collapsed foldout','pending');
