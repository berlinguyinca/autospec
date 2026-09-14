//! The `autospec convert` usage text (#4556 moved it here from the parent
//! module, which the repository's size guidance has kept flagging for
//! some time). It rides on every refusal that names an option.

pub(super) const USAGE: &str = "\
USAGE:
    autospec convert [--llm-root DIR] [--repo OWNER/NAME] [--base BRANCH]
                     [--held-file PATH] [--branch-prefix PREFIX]
                     [--free-slots N] [--apply] [--archive] [--json]
                     [--convert-ledger PATH] [--gate-registry PATH]
                     [--shared-llm-root] [ISSUE ...]

PLAN (default): enumerate $LLM/*/out/issue-*/changes.patch, select the patches
not already attempted (a live branch/PR, or a recorded HELD entry whose
re-gate still holds, disqualify), and report the plan. No mutations.

--apply: perform the real conversion of each selected patch — branch off
origin/<base>, full gate (fmt --check, build, clippy, test --no-fail-fast),
open a PR per passing patch, and record a HELD line (a JSON HoldRecord in
the ledger format below, never prose) for failures.

OPTIONS:
    --llm-root DIR        the agent-patch root (default: $LLM)
    --repo OWNER/NAME     the GitHub repo for PR liveness (default: gh)
    --base BRANCH         trunk to branch off origin/<base> (default: main)
    --held-file PATH      the HELD ledger (default: <llm-root>/held.txt)
    --branch-prefix P     conversion branch prefix (default: conv-)
    --free-slots N        the free agent slots the fleet reports: with it,
                          the pass alarms when the queue entries blocked on
                          conversion exceed the free slots — the precise
                          condition under which the fleet is wasting GPU
                          time (#4558)
    --gate-registry PATH  the per-repository gate registry (default:
                          $AUTOSPEC_GATE_REGISTRY, else
                          data/convert-gate-registry.json in the checkout).
                          The pass refuses a repository with no recorded
                          gate rather than guessing one (#4556)
    --shared-llm-root     the --llm-root is the shared parent of all pipelines
                          (the pass reached every pipeline by construction).
                          Without it the root is one pipeline and its siblings
                          are the coverage question (#4556)
    --apply               perform the conversion, not just the plan
    --archive             archive the named issues' patches (move, never
                          delete, to out/issue-N/superseded/) and release
                          their queue entries — the explicit exit for a
                          patch that can never convert. Requires explicit
                          ISSUE numbers; a bare sweep would free every
                          entry at once (#4558)
    --json                machine-readable plan
    --convert-ledger PATH one-off: convert a prose HELD ledger (lines of
                          `- <issue>  HELD <reason>`) into the JSON
                          HoldRecord lines the pass reads, one record per
                          issue, preserving the recorded reason; writes to
                          --held-file (default: <llm-root>/held.txt)
    ISSUE ...             restrict the pass to these issue numbers

BUFFER (reported on every plan and apply run): how many finished patches
are waiting (on disk, no live PR) and how many queue entries they block
(one per patch on disk; the dispatch guard holds each). With --free-slots
N an ALARM line when the blocked entries exceed the free slots: the
fleet's throughput is then limited by conversion, not compute (#4558).

HELD LEDGER (--held-file): one JSON line per hold — the serde form of
hold_memo::HoldRecord — fields issue, patch_key (patch mtime, seconds),
base_sha (trunk tip the hold was derived against), depends_on (file paths;
empty = the whole base), reason (what the gate reported: clippy=2, test X
FAILED, conflict in PATH — the report, not a sentence). Blank and # comment
lines are skipped. Prose belongs in reason (converted prose keeps its
sentence there) or in a sidecar keyed by issue; a HELD line is always
written in this format, never prose.";
