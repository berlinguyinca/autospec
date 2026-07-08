# AutoSpec Growth — autonomous, white-hat product promotion

**Status:** Design
**Date:** 2026-07-08
**Author:** berlinguyinca
**Feeds:** `/autospec-split` → `/autospec-run`

## Summary

Give AutoSpec a *product* — a website URL, a GitHub repo, and a positioning
statement — and it runs a perpetual **growth flywheel** that grows organic
search traffic, signups/subscriptions, GitHub stars, and public ratings
through **legitimate means only**. The cycle is
Research → Produce → Gate → Publish → Measure → Learn.

On-repo and on-site artifacts (blog posts, meta/schema fixes, comparison pages,
README badges) are produced and merged **fully autonomously** as PRs against the
product/site repo. Every action that touches a **third-party platform** (Reddit,
Hacker News, forums, directories, outreach email) is produced as a **draft** and
routed to a **human-approval control channel** — it is never auto-published by
default.

The design is deliberately built on top of existing AutoSpec machinery. Its
spine is a **fail-closed, white-hat ethics gate** with a hard-coded blocklist
that a repo may extend but never weaken.

### Non-goals

- No fake, incentivized-without-disclosure, or gated reviews; no rating/vote
  manipulation; no sockpuppets.
- No bot or fake signups to inflate user counts.
- No scraped-email cold spam, cloaking, private blog networks, or link schemes.
- No automation that violates a target platform's Terms of Service.
- Not a paid-ads manager (no ad-spend automation in v1).

## Skill shape

Mirrors the core family's `define` / `run` / conductor split. Three new
lock-step trio skills (`SKILL.md` + `codex/prompt.md` + `opencode/agent.md`,
bodies identical) plus one shared reference module.

| Skill | Role |
|---|---|
| `/autospec-grow-define` | Sync metrics → run growth researchers → adversarial-verify + ROI/severity rank → decompose into linked GitHub issues. Stops after decomposition and hands off. |
| `/autospec-grow-run` | Drain the growth backlog autonomously. Wraps `/autospec-run` for `growth:artifact` issues; routes `growth:outbound` issues to the approval pipeline. |
| `/autospec-grow` | Perpetual conductor. Walks a growth priority waterfall, calls define/run underneath, closes the measurement loop. Modeled on `autospec-autonomous` wrapping `define`/`run`. |
| `autospec-grow-shared` | `growth.yml` schema, the hard-coded ethics blocklist, and the measurement adapters. Modeled on `autospec-shared`. |

## Configuration — `.autospec/growth.yml`

Opt-in per repo (mirrors `autospec-fab`'s `.autospec/fab.yml`). A repo without
this file is inert to the growth skills.

```yaml
product:
  name: "Acme CLI"
  one_liner: "The fastest way to X"
  value_props: ["...", "..."]
  personas: ["indie devs", "platform teams"]
  competitors: ["CompA", "CompB"]
site:
  url: "https://acme.dev"
  repo_path: "."            # source of the site (for SEO PRs)
  framework: "astro"        # informs technical-SEO fixes
  sitemap_url: "https://acme.dev/sitemap.xml"
channels:
  technical_seo: true
  content: true
  outreach: true
  directories: true
targets:
  keyword_seeds: ["x cli", "fastest x"]
  directories: ["producthunt", "alternativeto", "awesome-x"]
  communities:                       # each carries its own self-promo rule + cadence cap
    - { platform: "reddit", where: "r/commandline", self_promo_rule: "1-in-10 rule", cadence_cap_per_week: 1 }
    - { platform: "hackernews", where: "Show HN", self_promo_rule: "genuine Show HN only", cadence_cap_per_week: 1 }
measurement:
  gsc_property: "sc-domain:acme.dev"
  analytics: { provider: "plausible", token_env: "PLAUSIBLE_API_TOKEN" }   # or ga4
  github_repo: "acme/cli"
  rank_source: "manual"        # or a SERP API adapter
approval:
  control_repo: "acme/growth"  # where approval issues live
  cadence_caps: { default_per_platform_per_week: 2 }
guardrails:
  extra_blocks: []             # MAY ADD hard-blocks; MAY NOT remove built-in defaults
```

**Secrets:** every credential is referenced by **env-var name only**
(`token_env`), never inline. A `growth.yml` containing an inline secret fails
schema validation and is caught by `autospec-secaudit`.

## The growth cycle (one conductor iteration)

### G0 — Sync & measure

Pull current metrics and compute deltas since the last cycle:

- **Google Search Console** — impressions, clicks, avg position per query/page.
- **Plausible / GA4** — traffic, sources, conversions to signup/subscription.
- **GitHub** — stars, forks, clones, referrer traffic (via API), plus a
  product-KPI hook (signups/MRR read from a webhook payload or CSV path named
  in config).
- **Rank + backlink** — target-keyword SERP positions and new/lost backlinks.

Deltas and raw snapshots are written to the **growth ledger** (reuses the
`autospec-explore-ledger` pattern: outcome memory, dynamic source weights, and a
learnings memo).

### G1 — Research (Tier-A fan-out, one lens per subagent)

Each lens proposes candidate growth tasks with `{roi, effort, evidence,
channel}`:

1. **Technical-SEO auditor** — crawl the site (via `autospec-playwright` /
   Claude-in-Chrome): Core Web Vitals, meta tags, canonical/hreflang, structured
   data (schema.org), sitemap/robots, broken links, index coverage.
2. **Keyword-gap researcher** — GSC queries ranking position 5–20 (quick wins)
   plus competitor keyword gaps.
3. **Content-opportunity researcher** — topic clusters, "vs"/comparison pages,
   tutorial and docs gaps.
4. **Community researcher** — subreddits, HN, forums, Discords, and
   awesome-lists where the product is *genuinely on-topic*, each annotated with
   that venue's self-promotion rule.
5. **Directory/listing researcher** — legitimate directories the product
   qualifies for (Product Hunt, AlternativeTo, awesome-list PRs).
6. **Backlink/partnership researcher** — guest-post targets and integration
   partners.

Fan-out budget per lens is scaled by that lens's ledger-derived weight from G5.

### G2 — Rank & decompose (Tier-A)

Adversarially verify candidates, **dedup against the ledger** (both
already-done and already-rejected — dedup against the full seen-set, not just
what shipped, so rejected items don't reappear each cycle), ROI/severity rank,
then decompose the survivors into GitHub issues using the standard AutoSpec
issue contract. Two issue classes:

- **`growth:artifact`** — produces an on-repo / on-site artifact. Drains through
  normal `/autospec-run` → PR → auto-merge.
- **`growth:outbound`** — produces a draft destined for a third-party platform.
  Drains through the outbound pipeline; **never auto-published**.

### G3 — Produce

- **Artifact issues** → `/autospec-grow-run` wraps `/autospec-run`. The
  implementer writes the content/code; a **content-quality gate** runs alongside
  the standard reviewer:
  - E-E-A-T signals present; no keyword stuffing; every factual/benchmark claim
    grounded or cited; brand voice enforced via `autospec-persona`; landing and
    comparison pages meet `frontend-design` quality.
  Then reviewer + `growth-ethics` gate + `autospec-secaudit` gate → PR →
  auto-merge to the site repo.
- **Outbound issues** → produce a draft: the post/email body, the target venue,
  and **the specific self-promo rule the draft satisfies**.

### G4 — Gate & publish

All produced items pass the **`growth-ethics` gate** (§ Guardrails) first, then:

- **Artifacts** — already gated by review + secaudit + ethics; publish by merge
  automatically.
- **Outbound** — each surviving draft becomes a GitHub issue in
  `approval.control_repo` labeled **`growth/needs-approval`**, containing the
  ready-to-paste content, the target link, and the rationale. A
  **PushNotification** fires when new drafts are ready. The human:
  - `growth/approved` — agent posts **only** for platforms explicitly opted in
    per-platform; otherwise it hands back a one-click-publish package and the
    human publishes. On publish, agent records `growth/published` + the live URL.
  - edits the issue body — agent re-gates and re-queues.
  - `growth/rejected` — recorded in the ledger's rejected set (never re-proposed).

This control channel doubles as the **live-steering surface** for the conductor
(mirrors `autospec-autonomous`): a human can add directives, pause, or redirect
between cycles.

### G5 — Learn & attribute

After a configurable lag window, correlate metric deltas (G0) with shipped
ledger items → derive **dynamic source weights** (which researcher lens actually
produces things that move rankings/traffic/signups) → write a learnings memo →
feed the weights back into the next cycle's G1 fan-out budget. This is the
`autospec-explore-ledger` recursive-self-improvement pattern applied to growth.

## Guardrails — the spine (white-hat, fail-closed)

A **blocking `growth-ethics` gate**: a Tier-A reviewer subagent backed by
deterministic pre-checks, paired with a 5-attempt adaptive-retry loop that feeds
findings back as directives (the standard AutoSpec LLM-validator pattern).

### Hard-coded blocklist (repo config may extend, never weaken)

The following are rejected unconditionally; `guardrails.extra_blocks` can only
**add** to this list:

- Fake, undisclosed-incentivized, or gated reviews; review solicitation that
  filters by expected sentiment.
- Rating / vote / star manipulation of any kind; sockpuppet or coordinated
  inauthentic accounts.
- Bot signups, fake signups, or any inflation of user/subscription counts.
- Scraped-email cold spam; unsolicited bulk outreach.
- Cloaking, doorway pages, private blog networks, paid/exchanged link schemes.
- Any automation that violates a target platform's Terms of Service.

### Enforced policies (deterministic where possible)

- **FTC disclosure** present on any sponsored/affiliate/incentivized content
  (deterministic presence check).
- **Per-platform cadence caps** honored, counted from the ledger (deterministic
  counter; gate refuses when over cap).
- **Genuine on-topic relevance** — outbound drafts must satisfy the target
  venue's self-promo rule recorded in config; no drive-by promotion.

### Fail-closed invariants

- Missing credentials, a gate error, or an un-parseable draft → **no publish**.
- The blocklist immutability is enforced by a validation test: a `growth.yml`
  that attempts to remove a built-in block fails closed.
- Every gate decision is logged with a machine-readable reason to the ledger.

## Reuse map

Per the ROI-check rule, every new component names a consumer that benefits
today, and existing skills are invoked rather than forked.

**Reused (no fork):**

- `autospec-run` / `autospec-review` / `autospec-secaudit` — drain and gate
  artifact issues.
- `autospec-define` decomposition — issue creation from ranked candidates.
- `autospec-persona` — brand voice for generated content.
- `autospec-explore-ledger` — growth ledger, source weights, learnings memo.
- `autospec-autonomous` — control-channel + live-steering pattern for the
  outbound approval queue.
- `autospec-playwright` / Claude-in-Chrome — site crawl for the technical-SEO
  audit and optional per-platform assisted posting.
- `autospec-doc` — tutorial/docs content generation.
- `frontend-design` — landing-page and comparison-page quality.

**New (each with a named consumer):**

- `growth.yml` schema — consumed by all three grow skills.
- The 6 researcher lenses — consumed by G1.
- The `growth-ethics` gate — consumed by G4 (and the per-artifact Phase-4 gate).
- The outbound pipeline + control-channel labels — consumed by G4.
- 4 measurement adapters (GSC, analytics, GitHub, rank/backlink) — consumed by
  G0 and G5.
- The attribution/re-weighting step — consumed by G5, feeding G1.

## Success metrics

- **Leading:** artifacts merged per cycle; keywords moved into top-10; indexed
  pages; CWV pass-rate; content-quality-gate pass-rate; outbound approval-rate.
- **Lagging:** organic clicks (GSC); signups/subscriptions (product KPI);
  GitHub stars; new backlinks; avg SERP position on the target keyword set.
- **Guardrail:** zero platform ToS strikes/bans; zero deceptive artifacts
  shipped (audited); 100% disclosure compliance; ethics-gate block-rate tracked
  (a rising rate signals mis-tuned researchers, not success).

## Testing

Repo convention is validation shell scripts (no language-level unit runner).
Each item is a script that passes after the change:

- `growth.yml` schema validation (required fields, env-var-only secrets).
- **Ethics-blocklist immutability test** — a config attempting to remove a
  built-in block fails closed.
- Lock-step trio diff checks for the 3 new skills (`SKILL.md` /
  `codex/prompt.md` / `opencode/agent.md` bodies identical) + skill-golden
  sha256 regeneration.
- Cadence-cap counter test — fixture ledger over cap → gate refuses.
- Disclosure-presence deterministic pre-check test.
- Per-lens fixture → decomposition-contract test (each researcher's output
  produces a valid `growth:artifact` / `growth:outbound` issue).
- Fail-closed test — missing creds / gate error → no publish path is reachable.

## Open questions

- Per-platform assisted posting: which venues (if any) get browser-automation
  posting on approval vs. always hand-back? Default is hand-back for all.
- Attribution lag window default (7 vs. 28 days) before G5 credits an item.
- Whether `/autospec-grow` should share the core autonomous conductor's quota /
  parking machinery or run its own.
