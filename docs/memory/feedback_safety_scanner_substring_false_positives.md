---
name: feedback_safety_scanner_substring_false_positives
description: autospec issue-safety rules substring-match, so ordinary prose auto-quarantines legitimate issues; diagnose with lint issue safety before rewording
metadata:
  type: feedback
---

The autospec issue-intent safety rules in `crates/autospec-core/src/claim/mod.rs` use
`contains_any` (~:1020), which is plain `text.contains` — **no word boundary, no
ordering, no proximity, whole document**. A correct `contains_any_word` (~:673) exists
in the same file and is not used by these rules.

Result: ordinary engineering prose auto-quarantines issues. Measured on 2026-09-02/03:

- `token` + `print` inside **"fingerprint"** and `write` inside **"writer"** → BLOCK
- `token` + "prints"/"shows" describing routing output → BLOCK
- `auth-secrets` (a fence-surface *name*) + "prints" → BLOCK. **An issue was blocked
  for naming a security control.**
- "disables" + "**auth**or" → `weaken-security-control` AMBIGUOUS
- "No migration, no discard path" → `production-or-infra-touch` on a *negated* mention
- "production managed-project API" (meaning production *code*, not an environment) → AMBIGUOUS

**Why:** the config's `production-or-infra-touch` regex has `\b` but the Rust
built-ins do not, and the rules are bag-of-words rather than verb-near-noun.

**How to apply:**
1. Never assume a quarantine is real. Diagnose first:
   `autospec lint issue safety --title "<t>" --actor <login> --json <body-file>`
   then grep the body for the rule's literal substrings *without* word boundaries —
   that is what actually matched.
2. `SAFETY_AMBIGUOUS` needs a trusted actor recorded **inside** the safety block, not
   just the `safety:reviewed` label. Format (exactly one decision + at most one actor):
   `- **decision:** \`SAFETY_PASS\`` then `- **reviewer:** \`<login>\``. A second
   `autospec-safety` block is `invalid_safety_markers` — insert into the existing one.
   `security:quarantined` and `autospec:needs-human` reject outright regardless.
3. An issue *about* one of these rules will self-block, because describing the defect
   requires the trigger words. #3113 and [[feedback_claim_session_binding_and_parallel_safe]]'s
   sibling #3488 both did. Implement out-of-band under operator authorization; judge
   acceptance on the stated criteria, never on the issue's own verdict.
4. `contains_credential_printing` was fixed by #3473. `contains_secret_exfiltration`
   was explicitly scoped out and still has the defect (#3488).
