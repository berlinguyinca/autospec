# Architecture fitness-function engine

## Purpose

AutoSpec now treats architectural constraints as continuous fitness functions:
automated, objective checks that run locally and in CI before merges. The design
follows the fitness-function framing from Ford, Parsons, Kua, and Sadalage's
*Building Evolutionary Architectures*, Chapter 2, which describes fitness
functions as architecture-level tests, and AWS's "Using Cloud Fitness Functions
to Drive Evolutionary Architecture," which emphasizes measurable values that
steer architecture evolution.

Sources:
- O'Reilly, *Building Evolutionary Architectures*, 2nd ed., Chapter 2 "Fitness Functions": https://www.oreilly.com/library/view/building-evolutionary-architectures/9781492097532/ch02.html
- AWS Architecture Blog, "Using Cloud Fitness Functions to Drive Evolutionary Architecture": https://aws.amazon.com/blogs/architecture/using-cloud-fitness-functions-to-drive-evolutionary-architecture/

## Components

- `.autospec/architecture-fitness.yml` declares each fitness function, threshold,
  metric, gate flag, and issue metadata.
- `scripts/architecture-fitness.sh run` evaluates the registry and exits non-zero
  when any gated threshold is breached.
- `--emit-issues <dir>` writes an `auto-implement` issue body containing the
  breached metric, threshold, and exact locations. `--file-issues` additionally
  attempts `gh issue create` from that generated body.
- `scripts/validate.sh` runs the gate and the Bats contract suite, so the default
  merge validation blocks regressions.
- `.github/workflows/architecture-fitness.yml` runs the same gate on pull requests
  and pushes to `main`.

## Initial fitness functions

| id | characteristic | threshold |
| --- | --- | --- |
| `financial_no_f64` | no binary floating point in money/price/qty/PNL paths | `0` matches |
| `shell_no_destructive_git` | automation avoids hard reset / force push | `0` matches |
| `rust_core_cli_direction` | core crate never depends on CLI crate | `0` matches |
| `async_safety_no_await_holding_lock` | no allowances suppressing `await_holding_lock` | `0` matches |
| `latency_budget_validate_fast` | no-op fast path stays under 50ms | `50ms` |

Additional dimensions from issue #1533 (coupling/instability, artifact size, and
compile-time ceilings) can be added by appending registry entries without changing
the runner interface.
