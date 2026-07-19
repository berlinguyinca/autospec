# GitHub Launch Post

AutoSpec turns a feature idea into specs, GitHub issues, validation gates, and reviewable pull requests for AI-assisted software teams.

Why it exists: AI coding agents can move quickly, but teams still need durable context, scoped issues, validation evidence, and maintainable review trails.

What to try first:

```bash
git clone https://github.com/berlinguyinca/autospec.git
cd autospec
autospec validate --fast
bash scripts/demo-recording.sh
cargo run --quiet --bin autospec -- doctor --json
```

Good first feedback: install friction, unclear docs, missing examples, and places where the workflow feels too heavy.
