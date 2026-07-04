# Architecture

AutoSpec is a set of portable skills, scripts, schemas, tests, and documentation that run inside supported AI coding harnesses. The repository is designed around explicit artifacts: specs, GitHub issues, PRs, validation output, and final reports.

```mermaid
flowchart TD
    A[User request] --> B[Define workflow]
    B --> C[Design spec]
    C --> D[Issue decomposition]
    D --> E[Model-fit classification]
    E --> F[Implementation monitor]
    F --> G[Pull request]
    G --> H[Validation scripts]
    G --> I[Reviewer pass]
    H --> J[Closeout report]
    I --> J
    J --> K{Release gates pass?}
    K -->|yes| L[Merge]
    K -->|no| M[Blocker report]
    L --> N[Story / docs / memory]
```

## Repository Layers

| Layer | Paths | Responsibility |
| --- | --- | --- |
| Skills | `skills/*/SKILL.md` | Workflow instructions loaded by Claude Code, Codex CLI, and OpenCode |
| Harness mirrors | `skills/*/codex/prompt.md`, `skills/*/opencode/agent.md` | Lock-step prompt bodies for supported harnesses |
| Scripts | `scripts/*.sh`, `scripts/*.mjs`, `scripts/*.py` | Deterministic gates, installers, prompt generation, QA helpers |
| Tests | `tests/**` | Bats, shell, and Python validation coverage |
| Schemas | `schemas/*.json` | Contracts for QA, fleet, fab, reliability, and related artifacts |
| Docs | `docs/**` | User, developer, architecture, config, and launch documentation |

## Data Flow

1. A user asks for a feature or release audit.
2. AutoSpec writes or consumes a design spec.
3. The spec is decomposed into GitHub issues.
4. Deterministic classification and optional model review assign work size labels.
5. Implementation runs in scoped worktrees and produces PRs.
6. Validation scripts, CI, QA, and review gates decide whether the PR can merge.
7. Closeout reports and story docs preserve what changed and why.

## Trust Model

AutoSpec treats agent claims as claims, not proof. Scripts and reviewers must re-read cited artifacts, run the relevant checks, and report gaps directly.

