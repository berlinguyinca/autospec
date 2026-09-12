# Configuration before storage: negative existence claims (issue #4192)

Asked "is GLM deployed?", one check listed a single storage directory, found
four Qwen files, and filed an issue stating GLM was benchmarked but never
deployed and DeepSeek never started. Both statements were false: 534 GB of
weights sat in a sibling directory that was never listed, and the model
catalogue (`models.tsv`) named both models plainly, with the GPU count,
context limits and slot count each needs. The catalogue and the runtime that
reads it (`pick-config.py`) disagreed — the catalogue had both models, the
runtime had neither — and that single divergence was the whole defect.

**To answer "does this system have X", read the configuration that would
reference X — not the storage where X might live.** Configuration is a smaller
search space than storage, it is authoritative about *intent* rather than
accident, and it is the thing the system itself consults. Storage answers "what
is on this disk", which is a different question and only becomes the right one
after configuration says X should exist and you are checking whether it does.

The scale is the tell: a check that can miss half a terabyte is not a weak
check, it is the wrong check. When the answer to "is X here?" comes back "no"
from an inspection of one location, the next step is to search where X would be
*declared*, before concluding anything. Concretely, before filing "X is not set
up":

1. `grep -rl X` the configuration files, not the data directories.
2. Confirm from at least two independent places — the catalogue and the
   runtime that reads it — because they can disagree, and the disagreement is
   usually the actual bug.
3. State in the issue *where you looked*. "Checked `$L/models`, found only
   Qwen" makes the gap in the evidence visible to a reviewer without knowing
   anything about the system.

This is the second instance of the same error (the first is #4167 at
file scale: `head -6`, then a claim about the whole file; see the
"Truncated views and negative claims" section above). A filed issue in
this pipeline is dispatched to an agent and implemented, so a wrong premise
becomes a wrong change — both times the correction had to chase a live issue
before an agent acted on it.

### Refuse and name what would tell you

The thing that caught this was a guard doing exactly the right thing:

```
FATAL: 'qwen3.8-27b-vision-q8' is not in models.tsv; refusing to guess GPUs
```

It had every input needed to guess a plausible GPU count and refused, naming
the file that would have authorised it. **A component that cannot determine a
safety-relevant parameter must refuse and name what would tell it** — the
opposite of the failure modes in #4135 and #4190, where a missing input
silently became a permissive default. A refusal that names its missing input is
both the fix and the diagnosis: it tells the operator which file to consult
instead of guessing for them.

### Fit is derived, not listed (#4244)

Scheduling eligibility is a property of the catalog plus the card, not a
property of the model name. **Answer "on which classes may this model be
scheduled?" by reading the catalog's `vram_mib` and comparing it to each
candidate card — never by a `case "$MODEL"` allowlist of names.** A list of
names drifts the moment the catalog gains a row; the derived answer cannot.
The fit test is exactly `card.vram_mib >= entry.vram_mib` — no margin is
invented at the guard, and a card whose VRAM is unmeasured (`vram_mib == 0`)
refuses with a distinct variant rather than being treated as fitting
everything (the permissive default is a lie) or silently dropped. A name the
catalog does not know is its own refusal (`FitCheck::Unknown`), never an
empty class list: a narrow probe's exit code is not a membership test.

Checkable in `autospec_core::fleet_models` (`FleetRegistry::eligible_classes`,
`GpuCard`, `FitCheck` — pure in-memory, no subprocess, so the node repo's
`worker.sh` / `pick-config.py` can adopt them as the single source of truth).
Tests: `crates/autospec-core/tests/fleet_models.rs`.
