# Specialist Agents

Specialist agents are deterministic role/checklist review systems. They produce findings and recommendations; they never merge, approve, bypass verifier, or bypass review quorum.

```bash
bash scripts/autospec-specialist-index.sh
bash scripts/autospec-assign-specialists.sh --dry-run --issue <number>
bash scripts/autospec-specialist-review-packets.sh --dry-run --issue <number>
bash scripts/autospec-run-specialist-review.sh --dry-run --issue <number>
```

All commands are operator-invoked and local. Confirm writes only local review packets or state files.
