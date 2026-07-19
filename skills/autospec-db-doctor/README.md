# autospec-db-doctor

Run diagnostics on the optional **autospec-db** telemetry module. Resolves the
`autospec-db` binary (PATH, then `~/.autospec/bin/autospec-db`), runs
`autospec-db doctor`, and maps each reported FAIL to a concrete operator fix.
Degrades gracefully — prints the install one-liner and stops — when the
binary is not installed. Never echoes a DSN.

Works on **Claude Code**, **OpenCode**, and **Codex CLI**.

## Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-db-doctor/install.sh | sh -s -- --harness all
```

Per-harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-db-doctor/install.sh | sh -s -- --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-db-doctor/install.sh | sh -s -- --harness opencode
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-db-doctor/install.sh | sh -s -- --harness codex
```

From a clone:

```bash
cd skills/autospec-db-doctor
./install.sh --harness all
```

## FAIL → fix mapping

| Doctor FAIL class | Mapped fix |
|---|---|
| No `db.conf` found | Run the `autospec-db` installer |
| Connect FAIL | Check DSN host/port/sslmode; pgbouncer reachability hint |
| Pending schema updates | Re-run the `autospec-db` installer |
| Spool nonzero | `autospec-db drain` |
| Anything else | Doctor line printed verbatim with `(no mapped fix — see autospec-db docs)` |

## Invocation

```bash
/autospec-db-doctor
```

No arguments. Dispatches to the installed `db-doctor.sh` helper in
`~/.autospec/scripts` (or `AUTOSPEC_SCRIPTS_DIR`), which owns binary
resolution, the doctor run, FAIL-to-fix mapping, and DSN redaction.

## Spec

`docs/specs/2026-07-10-autospec-db-telemetry-design.md`
