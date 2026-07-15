# Validation command matrix

`scripts/generate-validation-matrix.sh <repo> [<repo> ...]` emits JSON Lines: one row per validation category per repository.

Each repository always receives seven ranked categories, in this order:

1. `syntax`
2. `unit`
3. `integration`
4. `lint-typecheck`
5. `build`
6. `docs`
7. `smoke`

Rows use this shape:

```json
{"repo":"example","category":"unit","rank":2,"status":"found","severity":"covered","command":"npm run test:unit","source_path":"package.json","source_ref":"script:test:unit"}
```

When no command is discovered for a category, the row is still emitted with `status: "missing"`, `severity: "validation-gap"`, and null `command`, `source_path`, and `source_ref`. Missing validation is not reported as a product `feature-gap`.

The scanner reads command sources from `package.json`, `.github/workflows/*.yml`, `Makefile`, `Taskfile.yml`, and validation-relevant language manifests including `go.mod`, `pyproject.toml`, `requirements.txt`, `pom.xml`, `build.sbt`, `DESCRIPTION`, and `Dockerfile`. Commands cite the source path and the script, target, workflow step, or manifest key that justified the recommendation.

Repository validation runs this contract through the Rust entry point:

```bash
autospec validate --fast
```

The legacy shell validation dispatcher is intentionally not restored.
