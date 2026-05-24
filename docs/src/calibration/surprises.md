# Verifier Surprises Convention

The `surprises` field accepts human prose, but calibration only learns from
two structured prefixes. Free text is preserved in the database for review and
ignored by `skillnet calibration analyze`.

## Prefixes

Use one annotation per line:

```text
dead-weight: <trigger-name>: <note>
missed-signal: <trigger-name>: <note>
```

`dead-weight` means the section added by `<trigger-name>` was not useful for
this plan. When that trigger fired, the analyzer counts the row as a false
positive.

`missed-signal` means `<trigger-name>` should have added a useful section if
its threshold had been lower. When that trigger did not fire, the analyzer
counts the row as a false negative.

## Parser Semantics

The parser reads `surprises` line by line.

- Leading and trailing whitespace around the prefix and trigger are ignored.
- The first colon separates the prefix from the rest of the line.
- The next colon separates the trigger name from the note.
- Only exact prefixes `dead-weight` and `missed-signal` affect calibration.
- Lines without the structured shape are ignored by calibration.
- Multiple structured lines may appear in one `surprises` field.
- The same trigger may be annotated more than once when multiple aspects were
  dead weight or missed signals.

## Examples

False positive:

```text
dead-weight: long-serial-chain: serial-chain recovery added no useful work
```

False negative:

```text
missed-signal: infrastructure-spof: shared runner dependency needed a warning
```

Mixed human and structured notes:

```text
The run shipped, but the review checkpoint was too ceremonial.
dead-weight: mid-plan-rerouting: no reroute was needed
missed-signal: revendor-phase: lockfile churn needed compatibility notes
```

## Calibration Consequences

Structured surprises feed the trigger confusion counts documented in
[Calibration JSON Schema](json-schema.md).

`dead-weight` changes a fired trigger from helpful to false positive for that
plan. That increases `false_positives`, lowers `signal_rate`, and can support
a `raise-threshold` proposal when the trigger fires often enough.

`missed-signal` marks a non-fired trigger as a false negative for that plan.
That increases `false_negatives`, lowers `signal_rate`, and can support a
`lower-threshold` proposal when enough missed signals accumulate.

Plain prose has no calibration consequence. Use it for reviewer context, not
for threshold learning.
