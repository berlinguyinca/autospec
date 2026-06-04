<!-- Quality-differential fixtures (issue #1023, spec §5 D4). -->

# quality-diff fixtures

Fixtures consumed by `scripts/quality-differential.sh --step <name> --fixtures
<dir>` — the boilerplate guard. The harness runs a determinized step's
deterministic path on each fixture's `input`, then evaluates the output through
the fixture's own `assert.sh` (signal-property checks, **not** string equality).
The harness exits non-zero if **any** fixture fails its assert, and that
non-zero exit is the verdict: *the step must keep/restore its LLM path.*

## Fixture layout (per `<step>/<fixture>/` subdir)

| File         | Role                                                                                  |
|--------------|---------------------------------------------------------------------------------------|
| `input`      | Input file(s) for the step. For `refine-lenses` this is the raw operator prompt.       |
| `llm-golden` | Recorded LLM output for that input. Reference only; passed to `assert.sh` as `$2`.     |
| `assert.sh`  | Signal-property checks. Argv: `<det-output-file> <llm-golden-file>`. Exit 0 pass.      |

`assert.sh` MUST check signal properties, never string equality. The shipped
contract checks two properties:

1. **Content-token coverage** — the deterministic output references ≥2 distinct
   content tokens (≥4-char, non-stopword) drawn from the `llm-golden`, proving
   the output is about the right thing.
2. **No canned boilerplate** — the output contains none of the known
   template-only markers (e.g. the literal `What happens on empty input?`) that
   a padding path emits regardless of input.

## Steps / dirs

- `synth/` — self-test for the `synthetic` step (echoes input verbatim). A
  negative-path pair: `pass-signal/` passes, `fail-canned/` carries a canned
  marker and fails, so the harness exits non-zero over the pair.
- `refine-lenses/` — the real first consumer. `input` holds a raw operator
  prompt; the harness runs the actual `scripts/refine-prompt.sh --lens-mode
  deterministic` path on it. The deterministic lenses append canned boilerplate,
  so every fixture fails and the harness exits non-zero — the documented
  spec §7 KEEP-LLM verdict for refine lenses. The 3 prompts were recorded from
  real `refine-prompt.sh` lens runs.
