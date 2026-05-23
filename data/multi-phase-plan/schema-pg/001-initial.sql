CREATE TABLE schema_versions (
    version    INTEGER PRIMARY KEY,
    applied_at BIGINT NOT NULL
);

CREATE TABLE plans (
    id              TEXT PRIMARY KEY,
    created_at      BIGINT NOT NULL,
    name            TEXT NOT NULL,
    path            TEXT NOT NULL,
    flavor          TEXT NOT NULL,
    worktype        TEXT,
    phase_count     INTEGER NOT NULL,
    wave_count      INTEGER NOT NULL,
    max_chain_depth INTEGER NOT NULL,
    repo_spread     INTEGER NOT NULL,
    routing_dist    JSONB NOT NULL,
    shape_hash      TEXT NOT NULL,
    capture_reasons JSONB NOT NULL
);

CREATE TABLE triggers (
    id            BIGSERIAL PRIMARY KEY,
    plan_id       TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    input_value   DOUBLE PRECISION NOT NULL,
    threshold     DOUBLE PRECISION NOT NULL,
    fired         BOOLEAN NOT NULL,
    section_added TEXT
);
CREATE INDEX idx_triggers_name_fired ON triggers(name, fired);

CREATE TABLE phases (
    id           BIGSERIAL PRIMARY KEY,
    plan_id      TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    slug         TEXT NOT NULL,
    routing_tier TEXT NOT NULL,
    files        JSONB NOT NULL
);
CREATE INDEX idx_phases_plan ON phases(plan_id);

CREATE TABLE verifications (
    id                BIGSERIAL PRIMARY KEY,
    plan_id           TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    verified_at       BIGINT NOT NULL,
    elapsed_seconds   BIGINT,
    outcome           TEXT NOT NULL,
    phase_outcomes    JSONB NOT NULL,
    emergency_changes JSONB,
    surprises         TEXT
);

CREATE TABLE tags (
    plan_id TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    key     TEXT NOT NULL,
    value   TEXT NOT NULL,
    PRIMARY KEY (plan_id, key, value)
);
CREATE INDEX idx_tags_kv ON tags(key, value);

CREATE TABLE calibration_proposals (
    id                  BIGSERIAL PRIMARY KEY,
    proposed_at         BIGINT NOT NULL,
    trigger_name        TEXT NOT NULL,
    current_threshold   DOUBLE PRECISION NOT NULL,
    proposed_threshold  DOUBLE PRECISION NOT NULL,
    supporting_plan_ids JSONB NOT NULL,
    fire_rate           DOUBLE PRECISION NOT NULL,
    signal_rate         DOUBLE PRECISION NOT NULL,
    filter_tags         JSONB,
    decision            TEXT NOT NULL,
    decided_at          BIGINT,
    rationale           TEXT
);
CREATE INDEX idx_proposals_decision ON calibration_proposals(decision);
