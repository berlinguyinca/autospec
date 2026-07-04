# FAQ For Launch Comments

## Is this another autonomous coding agent?

No. AutoSpec is the workflow and validation layer around supported coding harnesses. It focuses on specs, issue decomposition, validation, review, and reporting.

## Why not just use GitHub issues?

GitHub issues are the storage layer. AutoSpec defines how an agent creates, classifies, implements, validates, and closes those issues.

## Why so much ceremony?

The ceremony is for work where auditability matters. For a one-line patch, plain chat may be faster.

## Can it run without GitHub?

You can read docs and run local validators without GitHub. The full workflow assumes GitHub issues and PRs.

## Is it safe?

It is safer than unstructured delegation, but it is not a guarantee. Use least-privilege credentials and review high-risk changes.

