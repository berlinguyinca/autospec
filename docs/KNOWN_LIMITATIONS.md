# Known Limitations

- Digital Twin v1 is heuristic and evidence-scored; it can miss dynamic runtime behavior.
- Structured rule coverage depends on the local Constitution and Baseline repositories.
- Worker v1/v2 remains bounded to docs/spec/metadata/test and low-risk code work.
- Autospec does not automatically perform dependency upgrades.
- Autospec does not automatically perform database migrations.
- Autospec does not change auth/security behavior without stuck/guidance review.
- Autospec does not auto-merge or self-approve PRs.
- Autospec does not install GitHub Actions, cron, schedulers, or background daemons.
- AI/NLAI and product scaffolds generate specs and issue drafts, not full app features.
- Policy repositories are local paths unless another source type is already configured.
- Reports avoid raw secret values, but operators should still review generated evidence before publishing.
