CREATE TABLE heuristic_thresholds (
    name       TEXT PRIMARY KEY,
    threshold  DOUBLE PRECISION NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by TEXT
);
