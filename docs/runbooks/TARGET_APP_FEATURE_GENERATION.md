# Target-App Feature Generation

Autospec can generate bounded target-app scaffolds only when the stack and recipe are known.

## Safe Examples

- Generate an in-app documentation center spec.
- Generate a Playwright viewport matrix test scaffold when Playwright is present.
- Generate AI provider/RAG/token-usage specs without database migrations.
- Generate NLAI capability-interface specs and pretty-rendering templates.
- Generate status-page, diagnostics, reporting, dependency-governance, and security/privacy docs.

## Unsafe Examples

- Create database migrations automatically.
- Change auth/security behavior automatically.
- Upgrade dependencies automatically.
- Rewrite app architecture broadly.
- Claim full AI/NLAI runtime implementation from specs alone.

Use decomposition when a target-app feature is too broad for one patch plan.
