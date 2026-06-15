# autospec-secaudit

Scans generated code for security vulnerabilities, secret/credential leaks,
IP/copyright/license violations, PII/data leaks, SQL injection, prompt
injection, and backdoors. Deterministic scanners (gitleaks, semgrep, trivy,
license-checker) feed a Tier-A LLM triage that confirms findings and adds the
ones scanners miss, then writes a report to `.autospec/secaudit.md` and files
must-fix survivors back into autospec as `auto-implement,security,priority:high`
issues.

Findings are grouped into six dimensions:

- `secrets` — credentials, API keys, tokens
- `vuln` — vulnerabilities, SQLi, command injection, backdoors
- `injection` — prompt injection (untrusted input reaching an LLM call)
- `license` — IP / copyright / license violations
- `pii` — PII / data leaks
- `cve` — dependency CVEs

## Entry points

- **Manual sweep:** `/autospec-secaudit` audits the working tree repo-wide.
- **Phase 4 gate:** invoked by the autospec-run implementer (via
  `security-remediation-loop.sh`) as a blocking per-PR gate.

See SKILL.md for full pipeline and enforcement-default semantics.

## Environment

- `AUTOSPEC_SEC_MAX_ROUNDS` — max remediation rounds in the Phase 4 gate
  (default `3`).
- `~/.autospec/no-secaudit.flag` — when present, disables the post-batch
  auto-fire after each `/autospec-run`.
- `AUTOSPEC_SKIP_ENSURE_TOOL_*` — skip auto-installing a specific scanner
  (degrades that concern to LLM-only with a loud WARN).

Report path: `.autospec/secaudit.md`.
