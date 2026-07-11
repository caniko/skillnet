ALTER TABLE skill_invocations ADD COLUMN harness TEXT NOT NULL DEFAULT 'claude';
ALTER TABLE skill_invocations ADD COLUMN source_event_id TEXT;
ALTER TABLE skill_invocations ADD COLUMN adapter_version TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE skill_invocations ADD COLUMN canonical_skill_name TEXT;
CREATE UNIQUE INDEX skill_invocations_source_event_idx
    ON skill_invocations(source_event_id)
    WHERE source_event_id IS NOT NULL;
CREATE INDEX skill_invocations_harness_idx ON skill_invocations(harness);
