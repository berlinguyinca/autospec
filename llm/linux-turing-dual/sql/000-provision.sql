-- Provisioning for the LLM gateway registry: three roles, one database.
--
-- HOW TO RUN IT
--   On the database host itself, as the local superuser over the unix socket,
--   with the three passwords supplied through the ENVIRONMENT (never argv, which
--   is visible in `ps`, and never \prompt, which reads the script's own
--   remaining lines as answers when invoked as `psql < file`):
--
--     QT_OWNER_PW=$(openssl rand -base64 24) \
--     QT_APP_PW=$(openssl rand -base64 24) \
--     QT_READ_PW=$(openssl rand -base64 24) \
--     sudo -u postgres env QT_OWNER_PW="$QT_OWNER_PW" QT_APP_PW="$QT_APP_PW" \
--          QT_READ_PW="$QT_READ_PW" \
--          psql -X -v ON_ERROR_STOP=1 -f 000-provision.sql
--
--   Requires psql 13 or newer for \getenv. Run against the DIRECT PostgreSQL
--   port, not a connection pooler: CREATE DATABASE cannot execute inside a
--   transaction, and a pooler in transaction mode will reject it.
--
--   Afterwards the database is reachable through the pooler with NO pooler
--   configuration change. Verified on this site: the pooler carries a wildcard
--   route (`* = host=localhost port=5432`), and its auth_query is centralised
--   via `auth_dbname`, resolving roles out of pg_shadow -- so a new database
--   needs no pool entry and a new role needs no auth entry.
--
-- WHERE THE PASSWORDS GO AFTERWARDS
--   Only the APPLICATION role's password belongs on the inference node, in the
--   credential store read by systemd LoadCredential -- never in a unit's
--   Environment=, which any local user can read via `systemctl show`. The owner
--   and read-only passwords belong in the operator's own password store; the
--   node never needs them.
--
-- ROLES
--   *_owner  owns the schema and runs migrations. Not used by the service.
--   *_app    the gateway's role. DML only, no DDL -- so a compromised node
--            cannot alter or drop the schema it reads.
--   *_read   read-only, for reporting.
--
--   NOTE for whoever writes the migrations: the default privileges below apply
--   only to objects created BY llmgw_owner. Migrations must therefore connect
--   as llmgw_owner. If a superuser creates the tables instead, llmgw_app
--   receives nothing, and it presents as a mystifying permission bug.

\set ON_ERROR_STOP on

\getenv owner_pw QT_OWNER_PW
\getenv app_pw   QT_APP_PW
\getenv read_pw  QT_READ_PW

\if :{?owner_pw}
\else
\echo 'QT_OWNER_PW is not set in the environment -- see the header above'
\quit
\endif
\if :{?app_pw}
\else
\echo 'QT_APP_PW is not set in the environment -- see the header above'
\quit
\endif
\if :{?read_pw}
\else
\echo 'QT_READ_PW is not set in the environment -- see the header above'
\quit
\endif

CREATE ROLE llmgw_owner LOGIN PASSWORD :'owner_pw' NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE ROLE llmgw_app   LOGIN PASSWORD :'app_pw'   NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE ROLE llmgw_read  LOGIN PASSWORD :'read_pw'  NOSUPERUSER NOCREATEDB NOCREATEROLE;

CREATE DATABASE llm_gateway OWNER llmgw_owner;

\connect llm_gateway

-- Deny by default: PUBLIC still holds CONNECT on a fresh database.
REVOKE ALL ON DATABASE llm_gateway FROM PUBLIC;
REVOKE ALL ON SCHEMA public FROM PUBLIC;

GRANT CONNECT ON DATABASE llm_gateway TO llmgw_owner, llmgw_app, llmgw_read;

CREATE SCHEMA IF NOT EXISTS llm AUTHORIZATION llmgw_owner;
GRANT USAGE ON SCHEMA llm TO llmgw_app, llmgw_read;

ALTER DEFAULT PRIVILEGES FOR ROLE llmgw_owner IN SCHEMA llm
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO llmgw_app;
ALTER DEFAULT PRIVILEGES FOR ROLE llmgw_owner IN SCHEMA llm
  GRANT SELECT ON TABLES TO llmgw_read;
ALTER DEFAULT PRIVILEGES FOR ROLE llmgw_owner IN SCHEMA llm
  GRANT USAGE, SELECT ON SEQUENCES TO llmgw_app;

-- Verification: three roles, none of them a superuser.
SELECT rolname, rolsuper, rolcreatedb, rolcreaterole
  FROM pg_roles WHERE rolname LIKE 'llmgw\_%' ORDER BY rolname;
