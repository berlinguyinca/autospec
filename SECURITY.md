# Security Policy

## Supported Scope

Security reports should focus on this repository's scripts, installers, skill instructions, schemas, examples, and documentation.

## Reporting

Please do not open a public issue for a suspected vulnerability. Use GitHub private vulnerability reporting if it is enabled for the repository, or contact the maintainer through the repository owner profile.

Include:

- Affected file or workflow.
- Reproduction steps.
- Expected impact.
- Whether credentials, tokens, production data, or destructive operations are involved.

## Handling Expectations

Maintainers will triage reports based on exploitability, blast radius, and whether the issue affects default workflows. Documentation clarity issues are welcome as public issues; credential leaks, command injection, unsafe installer behavior, and bypassable safety gates should be reported privately.

## Security Boundaries

AutoSpec can execute shell commands and interact with GitHub through the operator's environment. Do not run it in repositories or shells where the available credentials exceed the work you are willing to delegate.

The V72 Rust safety layer blocks common unsafe operation categories in safe mode and redacts common token/key shapes in text evidence. Treat that as a guardrail, not a sandbox: operators still own credential scope, shell environment, and production access.
