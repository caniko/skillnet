CREATE TABLE schema_versions (
    version    INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);

CREATE TABLE plans (
    id              TEXT PRIMARY KEY,
    created_at      INTEGER NOT NULL,
    name            TEXT NOT NULL,
    path            TEXT NOT NULL,
    flavor          TEXT NOT NULL,
    worktype        TEXT,
    phase_count     INTEGER NOT NULL,
    wave_count      INTEGER NOT NULL,
    max_chain_depth INTEGER NOT NULL,
    repo_spread     INTEGER NOT NULL,
    routing_dist    TEXT NOT NULL,    -- json
    shape_hash      TEXT NOT NULL,
    capture_reasons TEXT NOT NULL     -- json array
);

CREATE TABLE triggers (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id       TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    input_value   REAL NOT NULL,
    threshold     REAL NOT NULL,
    fired         INTEGER NOT NULL,    -- bool
    section_added TEXT
);
CREATE INDEX idx_triggers_name_fired ON triggers(name, fired);

CREATE TABLE phases (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id      TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    slug         TEXT NOT NULL,
    routing_tier TEXT NOT NULL,
    files        TEXT NOT NULL          -- json array
);
CREATE INDEX idx_phases_plan ON phases(plan_id);

CREATE TABLE verifications (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    plan_id           TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    verified_at       INTEGER NOT NULL,
    elapsed_seconds   INTEGER,
    outcome           TEXT NOT NULL,
    phase_outcomes    TEXT NOT NULL,   -- json
    emergency_changes TEXT,            -- json
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
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    proposed_at         INTEGER NOT NULL,
    trigger_name        TEXT NOT NULL,
    current_threshold   REAL NOT NULL,
    proposed_threshold  REAL NOT NULL,
    supporting_plan_ids TEXT NOT NULL,   -- json array
    fire_rate           REAL NOT NULL,
    signal_rate         REAL NOT NULL,
    filter_tags         TEXT,            -- json {k: v}
    decision            TEXT NOT NULL,   -- pending|accepted|rejected
    decided_at          INTEGER,
    rationale           TEXT
);
CREATE INDEX idx_proposals_decision ON calibration_proposals(decision);
