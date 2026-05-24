CREATE TABLE heuristic_thresholds (
    name       TEXT PRIMARY KEY,
    threshold  REAL NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_by TEXT
);
