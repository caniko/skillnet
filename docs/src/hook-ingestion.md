# Hook ingestion

`skillnet hook ingest` records metadata for matching configured Claude Code
skill invocations into `skill_invocations` (coverage depends on installed
event/matcher entries), without requiring a later `skillnet calibration record`
pass or mixing raw hook events into multi-phase-plan calibration tables. Other
harness adapters can use `skillnet usage record` against the same storage
contract.

## Quick start

1. Install the binary:

   ```sh
   cargo install skillnet
   ```

2. Point skillnet at your calibration database. For Postgres, set a supported
   URL variable:

   ```sh
   export DATABASE_URL='postgres://skillnet@localhost/skillnet'
   ```

   For SQLite, set the runtime data directory instead:

   ```sh
   export SKILLNET_DATA_DIR="$HOME/.local/share/skillnet"
   ```

3. Install Claude Code hooks and check status:

   ```sh
   skillnet hook install
   skillnet hook status
   ```

4. Run a Claude Code skill and inspect recent rows:

   ```sh
   psql "$DATABASE_URL" -c \
     'select skill_name, hook_event, started_at from skill_invocations order by id desc limit 5;'
   ```

For a catalog-wide, zero-filled report:

```sh
skillnet usage report --since 90d --format table
```

Invoke `skillnet usage record` from native harness activation events with a
stable `--event-id` so retries are ignored; do not parse transcripts or retain
prompts instead.

## Managed entries

`skillnet hook install` modifies only the target Claude Code settings file
(default `$HOME/.claude/settings.json`), preserving unrelated settings and
hooks, writing a one-time `.bak` alongside an existing file, and replacing it
atomically via a temporary file.

Managed entries carry `_skillnet_managed: true`. Rerunning
`skillnet hook install` replaces prior skillnet-managed entries for the selected
events, keeping the command idempotent for provisioning tools.

Remove only skillnet-managed entries with:

```sh
skillnet hook uninstall
```

## Home Manager

The Home Manager module installs `skillnet` and can render database config, but
does not edit Claude Code settings during activation. Run hook commands
explicitly after config changes:

```sh
skillnet hook install
skillnet hook status
```

Run `skillnet hook uninstall` explicitly to remove managed entries.

## Troubleshooting

- Without `--strict`, `skillnet hook ingest` prints handled database and
  payload ingestion errors to stderr and exits 0 so a broken logging path does
  not break interactive tool use (config loading and database-target resolution
  can still fail before non-strict handling runs).
- Pass `skillnet hook ingest --strict --event PostToolUse` in tests and CI when
  failures must be fatal.
- If `skillnet hook install` fails with a JSON parse error, fix comments or
  invalid JSON in the settings file first; the command refuses to rewrite
  invalid JSON.
- If `skillnet hook status` prints `not installed`, run `skillnet hook install`
  with the same `--settings` path Claude Code reads.
