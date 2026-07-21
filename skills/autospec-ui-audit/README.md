# autospec-ui-audit

The first whole-site UI audit slice produces a deterministic React Router route
inventory. It writes `route-inventory.json` and `route-inventory.md`, preserving
navigation, sitemap, and E2E mismatches instead of silently dropping them.

Install all harness surfaces and the deterministic helper:

```bash
bash skills/autospec-ui-audit/install.sh --harness all
```

Then invoke `/autospec-ui-audit`, or run the helper directly:

```bash
node "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-ui-route-inventory.mjs" \
  --repo "$PWD" --output-dir "$PWD/.autospec/ui-audit"
```

Exit codes: `0` is a reconciled inventory, `1` is an invariant/discovery failure,
and `2` is invalid input. Angular, Next.js, browser evidence, scoring, and remediation
are intentionally deferred.

Generated build trees and commented JSX are ignored. Registry query strings and
fragments reconcile by pathname. Uninstalling one harness retains the shared helper
until the last installed harness consumer is removed; `--harness all` removes it.
