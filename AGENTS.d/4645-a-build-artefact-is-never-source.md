# A build artefact is never source (issue #4645)

An 8.2 MB compiled ELF executable was committed to `inferweave-gateway` at the
repo root and merged. It sat there unignored until a routine `git merge`
printed `create mode 100755 iw-slurm-exporter` and the line was noticed by
accident.

## The mechanism

1. `go build ./cmd/iw-slurm-exporter` with no `-o` writes the executable next
   to the package — in the working tree.
2. `git add -A` sweeps it in.
3. Nothing rejects it: no `.gitignore` entry, no size check, no file-type check.
4. Review reads a diff, and a diff of an 8 MB binary is not something a
   reviewer reads.

`cargo build` has the same property via `target/`, which is conventionally
ignored — so the habit of trusting `git add -A` is reinforced everywhere except
the one place it fails.

## Why it matters beyond tidiness

- **It is permanent.** Removing the file from `HEAD` does not remove it from the
  pack. Every clone carries it forever unless published history is rewritten.
- **It is a trap, not just weight.** A stale executable in the tree is something
  a person can run. It does not rebuild when the source changes, so it silently
  diverges from the code beside it.
- **It defeats review.** The one artefact in the commit that nobody can read is
  the one that ships.

## The invariant

**A build artefact is never source. A commit that adds an executable or an
unreadable blob must be rejected before it reaches review.**

Two mechanical guards, neither of which requires anyone to remember anything:

1. **Reject tracked binaries at commit time.** A check that fails on a staged
   file which is a binary, or an executable (mode 100755) that is not a text
   script.
2. **Every repository must ignore what its own build produces.** For a Go
   module, that is each `cmd/<name>` basename at the repo root.

## The wider habit

`git add -A` is the root. An agent that stages by wildcard will eventually stage
something it did not author — a build output, a log, a core dump, an editor
swap file, a downloaded fixture. The staging step is where intent should be
expressed, and "everything that happens to be in the tree" is not an intent.

**Agents should stage the files they changed, by name.** Where a wildcard is
genuinely wanted, it must be paired with a guard that rejects what the wildcard
was not meant to catch.

## Where it is enforced

- `autospec-core::lint::implementation::binary_committed` — the `BINARY_COMMITTED`
  deterministic detector (Rust, the #4439/#4447 implementation-language rule).
  It fires on a binary file, or a mode-100755 file whose added lines have no
  shebang. Wired into `lint_implementation` and `commit_blocking_rules`; the
  corrective directive tells the agent to stage by name and add a `.gitignore`
  entry.
- `AGENTS.md` (Engineering standards) records the staging-by-name rule and the
  reason.
- The `DiffFile` model now carries the declared file mode from the diff header,
  so a policy can tell a 100755 executable from a 100644 source file without
  inspecting the host checkout.
