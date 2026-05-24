CREATE TABLE skill_invocations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    skill_name  TEXT NOT NULL,
    tool_name   TEXT,
    project_dir TEXT,
    started_at  TEXT NOT NULL,
    ended_at    TEXT,
    outcome     TEXT,
    plan_id     TEXT REFERENCES plans(id) ON DELETE SET NULL,
    payload     TEXT NOT NULL,
    hook_event  TEXT NOT NULL
);
CREATE INDEX skill_invocations_skill_name_idx ON skill_invocations(skill_name);
CREATE INDEX skill_invocations_session_idx ON skill_invocations(session_id);
CREATE INDEX skill_invocations_started_at_idx ON skill_invocations(started_at DESC);
