-- Provisioning for the LLM gateway registry. RUN BY THE OPERATOR OR THEIR DBA.
--
-- This project deliberately does NOT run this script. It requires a superuser,
-- and the superuser credential available to the tooling belongs to a production
-- database server. Review it, then run it yourself.
--
-- WHERE TO RUN IT
--   Against the DIRECT PostgreSQL port, not the connection pooler: CREATE
--   DATABASE cannot execute inside a transaction, and a pooler in transaction
--   mode will reject it. On the database host itself:
--
--     psql -h localhost -p <direct-port> -U <superuser> -d postgres \
--          -v ON_ERROR_STOP=1 -f 000-provision.sql
--
--   Once created, the database is reachable through the pooler with no pooler
--   configuration change -- verified: the pooler carries a wildcard route, and a
--   nonexistent database name is rejected by PostgreSQL itself, not by the
--   pooler.
--
-- INVOKE IT WITH -f, NOT WITH REDIRECTED STDIN
--   The \prompt directives below read from the TERMINAL. Running this as
--   `psql < 000-provision.sql` makes \prompt consume the script's own remaining
--   lines as answers, which silently sets a role's password to a line of SQL.
--   Always use `-f 000-provision.sql`.
--
-- PASSWORDS
--   Supplied interactively below so they never enter this file, your shell
--   history, or this public repository. Generate them with, e.g.
--       openssl rand -base64 24
--   Afterwards place ONLY the application role's password on the node, in the
--   credential store read by systemd LoadCredential -- never in a unit's
--   Environment=, which any local user can read via `systemctl show`.
--
-- ROLES
--   Three, mirroring the pattern already established on this server:
--     *_owner  owns the schema. Not used by the running service.
--     *_app    the gateway's own role. DML only, no DDL -- so a compromised
--              node cannot alter or drop the schema it reads.
--     *_read   read-only, for reporting and dashboards.

\set ON_ERROR_STOP on

\prompt 'owner role password: '       owner_pw
\prompt 'application role password: ' app_pw
\prompt 'read-only role password: '   read_pw

CREATE ROLE llmgw_owner LOGIN PASSWORD :'owner_pw' NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;
CREATE ROLE llmgw_app   LOGIN PASSWORD :'app_pw'   NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;
CREATE ROLE llmgw_read  LOGIN PASSWORD :'read_pw'  NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;

CREATE DATABASE llm_gateway OWNER llmgw_owner;

\connect llm_gateway

-- Deny by default. Without this, every role on the server can connect and read
-- the public schema, which on PostgreSQL 15+ is less permissive than it once
-- was but still grants CONNECT to PUBLIC.
REVOKE ALL ON DATABASE llm_gateway FROM PUBLIC;
REVOKE ALL ON SCHEMA public FROM PUBLIC;

GRANT CONNECT ON DATABASE llm_gateway TO llmgw_owner, llmgw_app, llmgw_read;

CREATE SCHEMA IF NOT EXISTS llm AUTHORIZATION llmgw_owner;

GRANT USAGE ON SCHEMA llm TO llmgw_app, llmgw_read;

-- Table privileges are granted by the migration that creates each table, and as
-- defaults here so a later migration cannot forget. The owner creates; the app
-- role reads and writes rows; the read role only reads.
ALTER DEFAULT PRIVILEGES FOR ROLE llmgw_owner IN SCHEMA llm
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO llmgw_app;
ALTER DEFAULT PRIVILEGES FOR ROLE llmgw_owner IN SCHEMA llm
  GRANT SELECT ON TABLES TO llmgw_read;
ALTER DEFAULT PRIVILEGES FOR ROLE llmgw_owner IN SCHEMA llm
  GRANT USAGE, SELECT ON SEQUENCES TO llmgw_app;

-- Verification. Should print the three roles, none of them a superuser.
SELECT rolname, rolsuper, rolcreatedb, rolcreaterole
  FROM pg_roles WHERE rolname LIKE 'llmgw\_%' ORDER BY rolname;
