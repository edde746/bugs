-- Counts how often a worker took the job. A conversion that kills the
-- process (out of memory) leaves the job 'processing' with an expiring
-- lease; the count lets the next claim give up instead of crash-looping.
ALTER TABLE dif_assemblies ADD COLUMN claims INTEGER NOT NULL DEFAULT 0;
