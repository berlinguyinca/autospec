-- Seed data for postgres-fixture integration target
CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    email TEXT NOT NULL,
    name TEXT,
    phone TEXT,
    ssn TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS events (
    id SERIAL PRIMARY KEY,
    user_id INT REFERENCES users(id),
    ip_address TEXT,
    event_type TEXT,
    ts TIMESTAMPTZ DEFAULT NOW()
);

INSERT INTO users (email, name, phone, ssn) VALUES
    ('alice@example.com', 'Alice Smith', '+1-555-0101', '123-45-6789'),
    ('bob@example.com',   'Bob Jones',  '+1-555-0102', '987-65-4321'),
    ('carol@example.com', 'Carol White','+1-555-0103', '111-22-3333');

INSERT INTO events (user_id, ip_address, event_type) VALUES
    (1, '10.0.1.5',  'login'),
    (2, '10.0.2.11', 'purchase'),
    (3, '10.0.3.99', 'logout');
