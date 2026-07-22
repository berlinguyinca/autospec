# Anonymizer output safety

The E2E clone anonymizer writes transformed CSV and SQL data to a temporary
`.anon` file. It replaces the source using a backup/restore sequence that also
works when the destination already exists on Windows. Stale `.anon` and
`.anonymize-backup` files are removed before processing and after failures, so
an interrupted run can be retried without consuming partial output.
