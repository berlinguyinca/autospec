# LinkedIn Launch Post

I am opening up AutoSpec: a spec-first workflow layer for teams experimenting with AI coding agents.

AutoSpec helps a developer move from product intent to a written spec, linked GitHub issues, model-fit labels, implementation PRs, validation gates, and closeout reports.

The goal is not to remove engineering judgment. The goal is to make agentic development easier to review, audit, and trust.

Start here:

```bash
git clone https://github.com/berlinguyinca/autospec.git
cd autospec
bash scripts/demo-recording.sh
cargo run --quiet --bin autospec -- doctor --json
```

I am looking for feedback from developers using Claude Code, Codex CLI, or OpenCode on real repositories.
