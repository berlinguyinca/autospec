# Tutorial Artifacts

Tutorial artifacts are generated from runtime feature metadata and screenshot/contact-sheet evidence.

```bash
bash scripts/autospec-generate-tutorial-artifacts.sh --dry-run --feature in-app-docs-center
bash scripts/autospec-generate-tutorial-artifacts.sh --confirm --feature in-app-docs-center
```

Autospec does not fabricate screenshots. Missing screenshots are called out as warnings and placeholders. Video and TTS generation are not implemented; narration scripts/specs are generated instead.
