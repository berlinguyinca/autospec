# Autospec Distribution — npm Package + npx Install + Marketplace Listings

**Status:** Draft design (2026-05-22)
**Scope:** Closes tracker #424. Lowers adoption friction from "git clone + read install.sh" to "npx autospec init" plus marketplace listings on Claude Code skill registry, Codex CLI prompts catalog, and OpenCode agent registry.

## 1. Goal & non-goals

### Goal
Make autospec installable in <2 minutes by a new operator without prior context. Three layers: (1) `@autospec/cli` npm package providing `npx autospec init / install / status`; (2) Homebrew formula for macOS; (3) marketplace listings (Claude Code, Codex, OpenCode) discoverable from each harness's skill browser.

### Non-goals
- Hosted service (no SaaS layer)
- Auto-update without user consent
- Replacing the existing `install.sh` (npx wraps it; doesn't replace)
- Windows-first packaging (npm package works on Windows-as-supported, Homebrew is macOS/Linux)

## 2. Architecture

```
@autospec/cli (npm)
  ├─ bin/autospec.js
  │   ├─ init       → bootstrap target repo (.autospec/test.yml + initial scopes)
  │   ├─ install    → copy/update skills + scripts to harness paths
  │   ├─ status     → list installed skills, current versions, drift
  │   ├─ upgrade    → re-fetch + reinstall latest
  │   └─ uninstall  → remove skills, leave .autospec/ + memory
  └─ scripts/       → wrappers calling autospec install.sh

Homebrew formula (macOS/Linux):
  brew install berlinguyinca/autospec/autospec
  → installs node + @autospec/cli + sets PATH

Marketplace listings:
  ~/.claude/skills/                  (Claude Code — already supported by install.sh)
  ~/.config/opencode/skills/         (OpenCode)
  ~/.codex/skills/                   (Codex CLI)
```

## 3. Component 1 — `@autospec/cli` npm package

Lives at `packages/cli/` in the autospec repo.

`packages/cli/package.json`:
```json
{
  "name": "@autospec/cli",
  "version": "0.1.0",
  "type": "module",
  "bin": { "autospec": "./bin/autospec.js" },
  "files": ["bin", "scripts", "skills"],
  "engines": { "node": ">=20" }
}
```

`packages/cli/bin/autospec.js` — top-level CLI dispatch. Each subcommand shells out to `scripts/<cmd>.sh` which in turn invokes the canonical `install.sh` paths.

**Publishing flow** (manual for first release; later CI-driven):
```bash
cd packages/cli
npm version patch / minor / major
npm publish --access public
```

## 4. Component 2 — Homebrew formula

`Formula/autospec.rb` in a separate tap repo `berlinguyinca/homebrew-autospec`:

```ruby
class Autospec < Formula
  desc "Multi-harness AI workflow suite: spec → issue tree → autonomous PRs"
  homepage "https://github.com/berlinguyinca/autospec"
  url "https://registry.npmjs.org/@autospec/cli/-/cli-0.1.0.tgz"
  sha256 "..."
  license "MIT"
  depends_on "node"

  def install
    system "npm", "install", "-g", "--prefix=#{libexec}", buildpath/"package.json"
    bin.install_symlink Dir["#{libexec}/bin/*"]
  end

  test do
    system bin/"autospec", "--version"
  end
end
```

## 5. Component 3 — Marketplace listings

Each harness has its own discovery mechanism:

### 5a. Claude Code skill registry
- `~/.claude/skills/` paths (already supported)
- Plus: submit to `anthropics/claude-skills-marketplace` (if such exists at ship time) with skill manifests linking to autospec repo

### 5b. Codex CLI prompts catalog
- `~/.codex/skills/` for autospec-* directories
- Plus: catalog entry referencing `https://github.com/berlinguyinca/autospec`

### 5c. OpenCode agent registry
- `~/.config/opencode/skills/` for autospec-* directories
- Plus: agent manifest in OpenCode's expected format

## 6. Component 4 — Quickstart docs

`docs/QUICKSTART.md` — 5-minute "first contact" guide:
1. Install: `npx autospec init` (one command)
2. Configure: edit `~/.autospec/model-profiles.yml` if needed
3. Try: `autospec define "build me a TODO list CLI"` against a fresh empty repo
4. Run: `autospec run` to watch the implementation pipeline
5. Done: PR landed; admire the result

Plus screencast or asciinema recording demonstrating the flow.

## 7. Component 5 — Public landing site

Lives under `docs/site/` and publishes to `berlinguyinca.github.io/autospec` via GitHub Pages action. Curates:
- Hero: 60-second autospec pitch
- Quickstart embed
- Generated docs (USER_MANUAL.md + ARCHITECTURE.md from autospec self)
- Pricing / cost expectations (token usage table from telemetry baseline)

## 8. Decomposition (4 phases)

| # | Phase | Size | Deps |
|---|---|---|---|
| D1 | `@autospec/cli` package skeleton + bin/init + bin/install (wrapping install.sh) | 1-2 PRs | none |
| D2 | bin/status + bin/upgrade + bin/uninstall + bats coverage | 1 PR | depends D1 |
| D3 | Homebrew formula in separate tap + CI release flow | 1 PR | depends D1 published to npm |
| D4 | QUICKSTART.md + asciinema demo + public landing site scaffold | 1-2 PRs | depends D2 |

D1-D2 priority:high. D3-D4 standard.

## 9. Testing

- Per-cmd bats: `autospec init` against tmpdir → expected `.autospec/test.yml` + initial scopes
- npm pack + install in clean sandbox → tarball loads cleanly
- Homebrew formula audit via `brew audit --strict`
- QUICKSTART manual walk-through against a fresh Node empty repo

## 10. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| Existing `install.sh` | live | `npx autospec install` wraps it |
| `bundle-static-context.sh` (#402) | live | not required by CLI, but available for any future inline LLM call |
| Telemetry telemetry.jsonl (#403) | live | `autospec status` reads it for current cache-hit-rate display |

### Out of scope
- Hosted service / SaaS
- Auto-update without consent
- Cross-distro Linux packaging (apt/yum/dnf) — npm + Homebrew cover most users; community can add others
- Web-based UI

## 11. Decision log

| Q | Decision | Rationale |
|---|---|---|
| npm or pip first? | npm | Higher overlap with target audience (Claude Code + Codex + OpenCode users) |
| Bundle skills inside npm package or fetch fresh? | Bundle in package | Reproducibility; version pinning |
| Homebrew or scoop? | Homebrew first | macOS-heavy user base; scoop in future if demand |
| Quickstart screencast? | Yes (asciinema) | Visual + text; copy-paste-able |

## 12. Open follow-ups

- Linux distro packaging (apt/yum/dnf)
- Windows-native install path (PowerShell module)
- VS Code extension wrapping the CLI
