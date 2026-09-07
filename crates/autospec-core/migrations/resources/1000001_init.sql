-- Subsystem: resources (epic #3185, issue #3188).
-- Version range 1xxxxxx is owned by the resource subsystem; no other
-- subsystem may emit a migration whose version is in this range (D7 shared
-- migration protocol). Table definitions are out of scope for the shared
-- database adoption; the ResourceLedger tables land with the ledger work.
SELECT 1;
