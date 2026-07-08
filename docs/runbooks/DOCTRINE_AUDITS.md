# Doctrine Audits

Autospec doctrine audits are local, operator-invoked checks that turn the broad Constitution vision into evidence, templates, and local backlog drafts. They do not publish issues, install GitHub Actions, create cron jobs, schedule background work, update dependencies, run migrations, or implement arbitrary target-app runtime features.

## Commands

| Command | Purpose | Primary report |
| --- | --- | --- |
| `bash scripts/autospec-architecture-governance.sh --dry-run` | ADRs, architecture map, design-pattern rationale, impact analysis | `.autospec/reports/architecture-governance.md` |
| `bash scripts/autospec-ui-ux-audit.sh --dry-run` | UI surface, design tokens, responsive/accessibility evidence, pretty output, raw JSON avoidance | `.autospec/reports/ui-ux-audit.md` |
| `bash scripts/autospec-playwright-evidence-audit.sh --dry-run` | Playwright config, viewport matrix, screenshots, visual diff, tutorial, white-screen flows | `.autospec/reports/playwright-evidence-audit.md` |
| `bash scripts/autospec-doc-artifact-audit.sh --dry-run` | README, user docs, RAG-ready docs, tutorials, PDFs, screencasts, TTS, docs drift | `.autospec/reports/doc-artifact-audit.md` |
| `bash scripts/autospec-reporting-analytics-audit.sh --dry-run` | Metrics, dashboards, reports, exports, chart libraries, statistics | `.autospec/reports/reporting-analytics-audit.md` |
| `bash scripts/autospec-ai-platform-audit.sh --dry-run` | Provider abstraction, OpenAI/Ollama, RAG, agents/tools, MCP, token/cost tracking | `.autospec/reports/ai-platform-audit.md` |
| `bash scripts/autospec-nlai-audit.sh --dry-run` | NLAI tools, data/SQL/file/report/workflow capability surface, permissions, explainability | `.autospec/reports/nlai-audit.md` |
| `bash scripts/autospec-diagnostics-audit.sh --dry-run` | Health, logs, metrics/tracing, white-screen diagnosis, incident reports, safe remediation | `.autospec/reports/diagnostics-audit.md` |
| `bash scripts/autospec-dependency-governance.sh --dry-run` | Manifests, lockfiles, library sprawl, migration docs, dependency policy | `.autospec/reports/dependency-governance.md` |
| `bash scripts/autospec-modernization-plan.sh --dry-run` | Local modernization backlog without changing dependencies | `.autospec/reports/modernization-plan.md` |
| `bash scripts/autospec-security-privacy-audit.sh --dry-run` | Threat model, secret references, permission model, audit logs, PII, retention, reviews | `.autospec/reports/security-privacy-audit.md` |
| `bash scripts/autospec-doctrine-audit.sh --dry-run --all` | Unified scorecard and local doctrine issue drafts | `.autospec/reports/doctrine-audit.md` |

## Recommended Sequence

```bash
bash scripts/autospec-spec-implementation-sweep.sh --dry-run --priority critical,high
bash scripts/autospec-doctrine-audit.sh --dry-run --all
bash scripts/autospec-spec-coverage.sh --dry-run
bash scripts/autospec-mvp-status.sh
```

## Output Policy

Markdown reports are the primary human output. JSON reports are pretty-printed and deterministic. Local issue drafts are written under `.autospec/backlog/doctrine/` and `.autospec/backlog/modernization/`.

## Safety

No GitHub Actions are added. No scheduler is added. All commands are operator invoked. Confirmed modes, where accepted, remain local-only unless an existing publishing command is explicitly invoked with `--confirm`.
