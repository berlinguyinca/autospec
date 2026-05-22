---
reverse_engineered: true
source_root: .
generated_at: 2026-05-22T16:42:57.781Z
commit: c5cd632
ai_reviewed:
  confidence: medium
---

# Architecture — reverse-engineered design

**Significant modules:** 100
**Trivial files (bubbled):** 50

## Module index

### scripts-autospec-review-audit

- **Language:** python
- **Files:** 1
- **Exports:** 19
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-ai-review-doc

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-gen-arch-diagram

- **Language:** javascript
- **Files:** 1
- **Exports:** 4
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-gen-assistant-prompt

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-gen-docs-from-spec

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### gen-docs-api-reference

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### gen-docs-architecture

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### gen-docs-user-manual

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### scripts-gen-llm-manifest

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-gen-screenshots

- **Language:** javascript
- **Files:** 1
- **Exports:** 11
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-loop-classifier-docs-extension

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### reverse-engineer-cluster

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### reverse-engineer-emit-spec

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### reverse-engineer-inventory

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-scan-doc-scope

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### tree-sitter-walk-walker

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### cli-main

- **Language:** go
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### internal-server

- **Language:** go
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### example-cli

- **Language:** java
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### example-lib

- **Language:** java
- **Files:** 1
- **Exports:** 2
- **Entry points:** 0
- **Significance:** has_exports

### example-server

- **Language:** java
- **Files:** 1
- **Exports:** 3
- **Entry points:** 2
- **Significance:** has_exports, cli_entry

### src-cli

- **Language:** typescript
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### src-lib

- **Language:** typescript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-server

- **Language:** typescript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-cli

- **Language:** python
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-lib

- **Language:** python
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-server

- **Language:** python
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-lib

- **Language:** rust
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-main

- **Language:** rust
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### src-server

- **Language:** rust
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-auth

- **Language:** python
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### lib-config

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 0
- **Significance:** has_exports

### lib-validator

- **Language:** python
- **Files:** 1
- **Exports:** 4
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-cli

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-parser

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-utils

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 0
- **Significance:** has_exports

### go-sample

- **Language:** go
- **Files:** 1
- **Exports:** 4
- **Entry points:** 2
- **Significance:** has_exports, cli_entry

### java-sample

- **Language:** java
- **Files:** 1
- **Exports:** 5
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### javascript-sample

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 3
- **Significance:** has_exports, cli_entry

### python-sample

- **Language:** python
- **Files:** 1
- **Exports:** 6
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### rust-sample

- **Language:** rust
- **Files:** 1
- **Exports:** 5
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### typescript-sample

- **Language:** typescript
- **Files:** 1
- **Exports:** 5
- **Entry points:** 0
- **Significance:** has_exports

### lib-invariants

- **Language:** typescript
- **Files:** 1
- **Exports:** 14
- **Entry points:** 0
- **Significance:** has_exports

### adapters-cargo-test

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### adapters-generic

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports, imported_by_3+

### adapters-go-test

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### adapters-jest

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### adapters-mocha

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### adapters-playwright

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### adapters-pytest

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### adapters-vitest

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### scripts-assertion-shift-classifier

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-assertion-shift-v2-buckets

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-behavior-taxonomy-check

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### contract-symmetry-interpolator

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### contract-symmetry-jsonpath-verifier

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 0
- **Significance:** has_exports

### contract-symmetry-ui-extractor

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### crawler-v2-affordance-verifier

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### crawler-v2-foldout-opener

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports, imported_by_3+

### scripts-error-signature

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-findings-generator

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-forbidden-url-check

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-function-presence

- **Language:** javascript
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### kinds-every-foldout-opens-all-nested

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### kinds-every-modal-returns-to-body-scroll

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### kinds-every-row-has-required-actions

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### kinds-every-visible-x-has-accessible-name

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### kinds-every-visible-x-is-y

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### invariants-run-structural

- **Language:** javascript
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### scripts-loop-classifier

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-mode-ii-postcheck

- **Language:** javascript
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### scripts-mode-ii-runtime-intercept

- **Language:** javascript
- **Files:** 1
- **Exports:** 2
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-network-intercept-inject

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### scripts-playwright-config-resolver

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### playwright-fixtures-touched

- **Language:** typescript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### scripts-quarantine

- **Language:** javascript
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### db-driver-jsonpath-store

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### db-driver-mysql

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### db-driver-postgres

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### db-driver-sqlite

- **Language:** javascript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### seed-shapes-verify-seeds

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### scripts-ui-crawler

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### window-contract-date-math

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports, imported_by_3+

### window-contract-request-recorder

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 1
- **Significance:** has_exports, cli_entry

### src-hello

- **Language:** go
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-hello-test

- **Language:** go
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### example-hello

- **Language:** java
- **Files:** 1
- **Exports:** 2
- **Entry points:** 0
- **Significance:** has_exports

### src-hello

- **Language:** python
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-test-hello

- **Language:** python
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-lib

- **Language:** rust
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-app

- **Language:** typescript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### src-server

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-app

- **Language:** typescript
- **Files:** 1
- **Exports:** 4
- **Entry points:** 0
- **Significance:** has_exports

### src-peak-detector

- **Language:** typescript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-app

- **Language:** typescript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### src-server

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### src-greeter

- **Language:** typescript
- **Files:** 1
- **Exports:** 3
- **Entry points:** 0
- **Significance:** has_exports

### assertion-shift-fixtures

- **Language:** javascript
- **Files:** 1
- **Exports:** 1
- **Entry points:** 0
- **Significance:** has_exports

### assertion-shift-run-parameterized

- **Language:** javascript
- **Files:** 1
- **Exports:** 0
- **Entry points:** 1
- **Significance:** cli_entry

### tests-test-autospec-review

- **Language:** python
- **Files:** 1
- **Exports:** 23
- **Entry points:** 0
- **Significance:** has_exports

## Trivial files

- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/test-targets/target-manifest-stale-bait/src/lib.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/test-targets/target-reverse-engineer-bait/src/router.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/test-targets/target-visual-stale-bait/src/dashboard.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/ai-review-doc.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/gen-arch-diagram.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/gen-docs.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/gen-llms.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/gen-screenshots.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/loop-classifier-docs.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/reverse-engineer.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/scan-doc-scope.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-shared/tests/unit/tree-sitter-walk.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/lib/index.ts`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/scripts/contract-symmetry/run-symmetry.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/scripts/crawler-v2/extended-crawler.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/scripts/window-contract/run-window.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/lang-matrix/jvm/src/test/java/com/example/HelloTest.java`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/lang-matrix/node/src/hello.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/lang-matrix/node/src/hello.test.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/target-clean-pass/tests/App.test.ts`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/target-failing-gap/tests/App.test.ts`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/target-greenwash-bait/tests/peak_detector.test.ts`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/test-targets/target-mode-ii-fixture/tests/app.test.ts`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/fixtures/lang/js/src/calculator.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/fixtures/lang/js/tests/calculator.test.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/fixtures/lang/js/tests/greeter.test.ts`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/fixtures/repos/greenwash-bait/src/calc.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/fixtures/repos/greenwash-bait/tests/calc.test.js`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/behavior-taxonomy-check.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/error-signature.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/forbidden-url-check.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/loop-classifier.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/mode-ii/backup-drivers.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/mode-ii/postcheck.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/mode-ii/preflight.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/mode-ii/quarantine.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/mode-ii/runtime-intercept.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/ui-crawler.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/date-math.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/extended-crawler.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/kinds/every-foldout-opens-all-nested.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/kinds/every-modal-returns-to-body-scroll.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/kinds/every-row-has-required-actions.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/kinds/every-visible-x-has-accessible-name.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/kinds/every-visible-x-is-y.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/lib-import.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/run-structural.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/run-symmetry.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/run-window.test.mjs`
- `/private/tmp/wt-feat-autospec-docs-phase10c/skills/autospec-test/tests/unit/v2/verify-seeds.test.mjs`

## Summary

> *Auto-generated by reverse-engineer pipeline. Edit this section to describe the system architecture.*
