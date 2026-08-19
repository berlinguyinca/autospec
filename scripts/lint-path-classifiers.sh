#!/usr/bin/env bash
# lint-path-classifiers.sh — how lint-implementation.sh files a changed path.
#
# Sourced, not executed. Extracted from lint-implementation.sh because that file is
# past the size ratchet and may not grow, and because these three predicates decide
# which detectors see which files: getting one wrong silently points a whole family
# of rules at the wrong half of the codebase.
#
# Conventions: no set -e reliance, if/then/fi for one-sided conditionals, bash 3.2
# safe (this repo's macOS floor).

# is_test_file PATH — returns 0 if path is under tests/
is_test_file() {
    # Nested as well as root, for the same reason is_doc_file is: 826 of this
    # repo's test files live outside the root tests/ tree -- 381 under
    # crates/autospec-cli, 75 under skills/autospec-shared, 51 under
    # skills/autospec-test. Anchoring the glob pointed every test-quality
    # detector at the smaller half of the codebase: ASSERTION_DENSITY and the
    # VACUOUS_* family silently skipped those 826 files, while TODO_LEFT and
    # DOC_OUT_OF_SYNC treated them as production source and fired on them.
    case "$1" in
        tests/*|*/tests/*) return 0 ;;
        *) return 1 ;;
    esac
}

# is_fixture_file PATH — returns 0 if the path holds captured data, not live code.
# A .diff under tests/fixtures/ exists precisely to contain code that violates a
# rule, so scanning its body reports the fixture as the violation. DOC_OUT_OF_SYNC
# and the density scanner already skip *.diff; the quality detectors that read
# added lines need the same exemption. SECURITY deliberately does NOT use this: a
# leaked key is a leaked key wherever it sits.
is_fixture_file() {
    case "$1" in
        *.diff|*.patch) return 0 ;;
        *) return 1 ;;
    esac
}

# is_doc_file PATH — returns 0 if path is a doc file
is_doc_file() {
    # Nested as well as root: this repo documents subprojects in place --
    # llm/*/README.md, llm/*/docs/*.md, .autospec/*/README.md -- and 63 of its 64
    # README files are not at the root. Anchoring the glob made every one of them
    # invisible, so a change documented in the right place still tripped
    # DOC_OUT_OF_SYNC and the only way to satisfy the gate was to touch an
    # unrelated root doc. SKILL.md was already matched both ways.
    case "$1" in
        README*|*/README*) return 0 ;;
        AGENTS.md|*/AGENTS.md) return 0 ;;
        docs/*|*/docs/*) return 0 ;;
        SKILL.md|*/SKILL.md) return 0 ;;
        skills/*/prompts/*.md|skills/*/references/*.md) return 0 ;;
        *) return 1 ;;
    esac
}
