# Truncated views and negative claims (issue #4167)

An agent inspected a runner's `out/issue-N/status.txt` with `head -6`, concluded
"records `issue`, `status`, `agent_rc`, `agent_secs`, `changed_files` — and no
SHA", and filed that as an invariant in a queued issue. The file has 18 lines;
line 17 is `base_sha=...`, present in every patch on the cluster. Left alone, an
implementer would have added a duplicate field. **The blast radius of a bad issue
in an autonomous pipeline is a code change, not a note**: a wrong invariant does
not stay wrong on paper, it becomes a duplicate field, a second source of truth,
and the divergence between them becomes a later bug.

- **Read the whole artefact before asserting what it lacks.** A negative claim
  about a file requires the whole file: `cat`, or an explicit `grep -c` for the
  thing claimed missing. `head`/`tail`/`| head -N` support positive claims only.
  Where output is genuinely too large, the negative claim must be made by search
  (`grep -L`, `grep -c`), never by eyeballing a window. Absence within a window
  is not absence.
- **An invariant that asks for a new field must first demonstrate its absence.**
  Include the check in the issue: `grep -c '^base_sha=' status.txt` → 0. Filing
  "the system must record X" is a request for a schema change; the evidence bar
  is showing X is not already there, not noticing it wasn't in what you happened
  to look at.
- **Prefer "use the existing field" over "add a field" whenever both are
  possible.** Two fields carrying the same fact will drift, and nothing will say
  which is authoritative.
- **Correct a filed issue the moment its premise fails, and say which invariants
  are withdrawn by number.** An issue in the queue is live work. A correction
  buried in prose is not enough — name the invariant that is withdrawn so an
  implementer cannot miss it.

And the measurement the invariant was requested for, once possible with the
existing field, contradicted the reasoning that demanded it: the patches that
landed were further behind main (median 189 commits) than the conflict-held ones
(130). Measuring instead of arguing falsified the hypothesis rather than
confirming it — the ordinary outcome when the data is read whole.
