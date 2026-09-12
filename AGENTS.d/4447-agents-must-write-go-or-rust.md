# Agents must write Go or Rust (issue #4447)

A gate that rejects without redirecting makes backlog, not code. Agents still
default to shell, the ratchet refuses the patch, and the work becomes a held
patch instead of a merged one — one recent batch was 710 lines of shell and 0
lines of Rust, every line written by an agent that had no instruction telling
it what language to write. The redirect has to reach the agent at the point it
chooses a language: the issue text, before it writes anything.

## The rule

**Every issue that will be implemented by an agent names its implementation
language, and the only admissible answers are Go or Rust.**

Shell remains admissible only for what genuinely cannot be a binary: a cron
line, and the few lines needed to launch a compiled artifact. Neither is a
place for logic.

## The mapping (settled, not re-derived per issue)

- `metabolomics-us/*` -> **Go**
- `InferWeave/*`, `berlinguyinca/autospec` -> **Rust**

Resolution lives in `crates/autospec-core/src/implementation_language.rs`
(`implementation_language(repo)`), the single source of truth: the issue
generator, the continuation-children generator, and the shell ratchet all
resolve the same answer. An issue generator that cannot resolve a language
refuses to file the issue rather than guess — an issue that reaches an agent
without a named language is a defect, not a prompt to improvise.

## The portability reason (not negotiable by taste)

The project is too complex for shell, **and it may be executed under different
operating systems**. Shell encodes host assumptions — GNU vs BSD flag
spellings, `/proc`, `readlink -f`, `mktemp` syntax, which `grep` is on `$PATH`
— that do not survive the move. A compiled Go or Rust binary carries its
behaviour with it. Nearly every defect in the session's incident log came from
shell semantics a compiler rejects or a type prevents; none are expressible in
the Rust or Go beside them.
