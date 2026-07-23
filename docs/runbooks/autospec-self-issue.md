# Autospec self-issue filing

`skills/autospec-shared/scripts/autospec-self-issue.sh` converts a JSON finding
into a deterministic issue title and body. It defaults to the autospec
repository, supports `--repo` overrides and `--dry-run`, and rate-limits the
normalized category/summary key through its local cache. Missing `gh` or an
offline create fails closed without prompting.
