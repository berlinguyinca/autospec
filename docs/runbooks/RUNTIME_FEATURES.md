# Runtime Features

Runtime feature generation creates bounded target-app shell or partial slices for recognized stacks. It is disabled for supervisor execution by default and remains operator-invoked.

```bash
bash scripts/autospec-detect-stack-profile.sh
bash scripts/autospec-runtime-adapter-index.sh
bash scripts/autospec-feature-slice-index.sh
bash scripts/autospec-runtime-implementation-plan.sh --dry-run --feature in-app-docs-center
bash scripts/autospec-generate-runtime-feature.sh --dry-run --feature in-app-docs-center
bash scripts/autospec-generate-runtime-feature.sh --confirm --feature in-app-docs-center
bash scripts/autospec-sync-runtime-metadata.sh --dry-run
bash scripts/autospec-verify-runtime-feature.sh --dry-run --feature in-app-docs-center
```

Shell means navigable UI/API/CLI shape exists. Partial means limited local behavior works with safe placeholder/static/config data. Complete means end-to-end behavior is verified. v1 mostly generates shell or partial slices.

No dependencies, migrations, auth/security changes, scheduler, GitHub Actions, auto-merge, or self-approval are added.
