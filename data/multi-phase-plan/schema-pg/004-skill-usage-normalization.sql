ALTER TABLE skill_invocations ADD COLUMN IF NOT EXISTS harness TEXT NOT NULL DEFAULT 'claude';
ALTER TABLE skill_invocations ADD COLUMN IF NOT EXISTS source_event_id TEXT;
ALTER TABLE skill_invocations ADD COLUMN IF NOT EXISTS adapter_version TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE skill_invocations ADD COLUMN IF NOT EXISTS canonical_skill_name TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS skill_invocations_source_event_idx
    ON skill_invocations(source_event_id)
    WHERE source_event_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS skill_invocations_harness_idx ON skill_invocations(harness);
