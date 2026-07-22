# Anonymizer output safety

The E2E clone anonymizer writes transformed CSV and SQL data to a temporary
`.anon` file. It replaces the source using a backup/restore sequence that also
works when the destination already exists on Windows. On startup and failure,
stale `.anon` files are removed; a leftover `.anonymize-backup` is restored when
the destination is missing and removed only when the destination already
exists. This makes interrupted runs retryable without deleting the sole source
copy. The replacement is recovery-safe, but not a cross-platform transactional
filesystem primitive.
