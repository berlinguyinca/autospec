## Goal

Prevent the autonomous conductor from idling forever: it treats an absent status surface as pending CI and skips every drain cycle indefinitely.

## Tests required

- [ ] No-status pending main is handled without an idle loop.
