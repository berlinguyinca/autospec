-- Subsystem: core (AS-AEO-001 Epic 2, issue #3188).
-- Version range 2xxxxxx is owned by the AS-AEO-001 persistence layer; no
-- other subsystem may emit a migration whose version is in this range
-- (D7 shared migration protocol). A real marker so Epic 2 can add tables to
-- the shared database without colliding with resource migrations.
SELECT 1;
