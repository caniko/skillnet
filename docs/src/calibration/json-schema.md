# Calibration JSON Schema

`skillnet calibration analyze --format json` emits one JSON object. The analyze
JSON schema is versioned independently from `.calibration.json` sidecars.

From `skillnet` 0.4.0, this output is SemVer-stable:

- Removing a field, renaming a field, changing a field type, changing enum tag
  style, or changing existing enum values requires a major version bump.
- Adding a field is allowed in a minor release.
- Consumers should treat unknown `verdict` strings as `monitor` and unknown
  object fields as additive data.

The current `schema_version` is `1`.

## Top-Level Object

| Field                 | Type                         | Optional | Meaning                                                                                       |
| --------------------- | ---------------------------- | -------- | --------------------------------------------------------------------------------------------- |
| `schema_version`      | integer                      | no       | Analyze JSON schema version. This is `1` for the 0.4.0 schema.                                |
| `analyzed_at`         | string                       | no       | Database `CURRENT_TIMESTAMP` when the report was created, formatted as `YYYY-MM-DD HH:MM:SS`. |
| `min_n`               | integer                      | no       | Minimum fire count required before `signal_rate` is considered actionable.                    |
| `dataset_size`        | integer                      | no       | Number of verified trigger rows included after trigger and tag filters.                       |
| `filter_tags`         | array of tag filters         | no       | Back-compatible copy of the tag filters applied to the run.                                   |
| `filter_tags_applied` | array of tag filters         | no       | Canonical list of tag filters applied to the run.                                             |
| `triggers`            | array of trigger analyses    | no       | One row per trigger present in the filtered dataset.                                          |
| `proposals`           | array of threshold proposals | no       | Threshold changes the analyzer would recommend from the current data.                         |
| `skew_warnings`       | array of skew warnings       | no       | Per-tag skew warnings. Empty when filters are applied.                                        |

Tag filter objects have this shape:

| Field   | Type   | Optional | Meaning                                  |
| ------- | ------ | -------- | ---------------------------------------- |
| `key`   | string | no       | Tag key, such as `flavor` or `worktype`. |
| `value` | string | no       | Required tag value.                      |

## Trigger Analysis

Each object in `triggers` has this shape:

| Field                 | Type                            | Optional | Meaning                                                                                                                                                                                   |
| --------------------- | ------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `trigger`             | string                          | no       | Trigger name.                                                                                                                                                                             |
| `fires`               | integer                         | no       | Count of verified rows where the trigger fired.                                                                                                                                           |
| `misses`              | integer                         | no       | Count of verified rows where the trigger did not fire.                                                                                                                                    |
| `fire_rate`           | number                          | no       | `fires / (fires + misses)`, or `0.0` for an empty denominator.                                                                                                                            |
| `signal_rate`         | number or null                  | no       | Signal score once `fires >= min_n`; otherwise `null`.                                                                                                                                     |
| `verdict`             | string                          | no       | Human-readable verdict. Stable values include `hold`, `lower threshold (...)`, `raise threshold (...)`, and `n=... < min-n=...`. Consumers should map unknown values to monitor behavior. |
| `default_threshold`   | number or null                  | no       | Catalog default threshold when the trigger is catalog-backed.                                                                                                                             |
| `current_threshold`   | number or null                  | no       | Active threshold from an override or catalog default. Falls back to the newest recorded row threshold when no catalog threshold exists.                                                   |
| `threshold_source`    | threshold source object or null | no       | Source of `current_threshold` when the trigger is catalog-backed.                                                                                                                         |
| `true_positives`      | integer                         | no       | Fired, shipped or partial, and not marked `dead-weight`.                                                                                                                                  |
| `false_positives`     | integer                         | no       | Fired but not helpful, including structured `dead-weight` rows.                                                                                                                           |
| `false_negatives`     | integer                         | no       | Did not fire but had `missed-signal` or a matching emergency-change hint.                                                                                                                 |
| `true_negatives`      | integer                         | no       | Did not fire and had no failure-mode signal.                                                                                                                                              |
| `supporting_plan_ids` | array of strings                | no       | Plan ids supporting a proposed verdict, empty for holds and min-n guards.                                                                                                                 |

The signal-rate formula is:

```text
signal_rate = (true_positives - false_positives - false_negatives)
              / max(1, true_positives + false_positives + false_negatives)
```

## Threshold Source

`threshold_source` is a tagged object with `type` as the tag.

Default threshold:

```json
{ "type": "default" }
```

Override threshold:

```json
{
  "type": "override",
  "updated_at": "2026-05-24 12:00:00",
  "updated_by": "proposal:42"
}
```

`updated_by` is a string or `null`.

## Threshold Proposal

Each object in `proposals` has this shape:

| Field                 | Type             | Optional | Meaning                                                          |
| --------------------- | ---------------- | -------- | ---------------------------------------------------------------- |
| `trigger`             | string           | no       | Trigger name.                                                    |
| `action`              | string           | no       | `lower-threshold` or `raise-threshold`.                          |
| `current_threshold`   | number           | no       | Active threshold at analysis time.                               |
| `proposed_threshold`  | number           | no       | Candidate replacement threshold.                                 |
| `fire_rate`           | number           | no       | Fire rate at analysis time.                                      |
| `signal_rate`         | number           | no       | Signal rate at analysis time.                                    |
| `supporting_plan_ids` | array of strings | no       | False-negative ids for lowering, false-positive ids for raising. |

## Skew Warning

Each object in `skew_warnings` has this shape:

| Field                | Type    | Optional | Meaning                                                     |
| -------------------- | ------- | -------- | ----------------------------------------------------------- |
| `trigger`            | string  | no       | Trigger name.                                               |
| `tag_key`            | string  | no       | Skew axis. Current analyzer checks `flavor` and `worktype`. |
| `tag_value`          | string  | no       | Tag band showing skew.                                      |
| `global_signal_rate` | number  | no       | Signal rate across the unfiltered dataset.                  |
| `band_signal_rate`   | number  | no       | Signal rate within the tag band.                            |
| `band_fires`         | integer | no       | Fire count in the tag band.                                 |
| `message`            | string  | no       | Human-readable warning.                                     |

## Example

```json
{
  "schema_version": 1,
  "analyzed_at": "2026-05-24 12:00:00",
  "min_n": 1,
  "dataset_size": 3,
  "filter_tags": [],
  "filter_tags_applied": [],
  "triggers": [
    {
      "trigger": "long-serial-chain",
      "fires": 2,
      "misses": 1,
      "fire_rate": 0.6666666666666666,
      "signal_rate": 0.3333333333333333,
      "verdict": "hold",
      "default_threshold": 4.0,
      "current_threshold": 4.0,
      "threshold_source": {
        "type": "default"
      },
      "true_positives": 2,
      "false_positives": 0,
      "false_negatives": 1,
      "true_negatives": 0,
      "supporting_plan_ids": []
    }
  ],
  "proposals": [],
  "skew_warnings": []
}
```
