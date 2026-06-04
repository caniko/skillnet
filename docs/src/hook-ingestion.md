# Hook ingestion

`skillnet hook ingest` records Claude Code hook payloads into the
`skill_invocations` table. This captures every configured skill invocation as
it happens, without requiring a later `skillnet calibration record` pass and
without mixing raw hook events into the multi-phase-plan calibration tables.

## Quick start

1. Install the binary:

   ```sh
   cargo install skillnet
   ```

2. Point skillnet at your calibration database. For Postgres, set one of the
   supported URL variables:

   ```sh
   export DATABASE_URL='postgres://skillnet@localhost/skillnet'
   ```

   For SQLite, select the runtime data directory instead:

   ```sh
   export SKILLNET_DATA_DIR="$HOME/.local/share/skillnet"
   ```

3. Install the Claude Code hook entries and confirm status:

   ```sh
   skillnet hook install
   skillnet hook status
   ```

4. Run any Claude Code skill, then inspect recent rows:

   ```sh
   psql "$DATABASE_URL" -c \
     'select skill_name, hook_event, started_at from skill_invocations order by id desc limit 5;'
   ```

## Managed entries

`skillnet hook install` edits only the selected Claude Code settings file,
defaulting to `$HOME/.claude/settings.json`. It preserves unrelated settings
and hook entries, creates a one-time `.bak` next to an existing settings file,
and writes through a temporary file before renaming.

Managed hook entries are marked with `_skillnet_managed: true`. Re-running
`skillnet hook install` removes previous skillnet-managed entries for the
selected events, then writes fresh entries. That makes the command idempotent
and safe to run from provisioning tools.

To remove only skillnet-managed entries:

```sh
skillnet hook uninstall
```

## Home Manager

The Home Manager module installs the `skillnet` binary and can render database
configuration, but it does not edit Claude Code settings during activation. Run
the hook workflow explicitly after changing configuration:

```sh
skillnet hook install
skillnet hook status
```

Run `skillnet hook uninstall` explicitly when you want to remove managed entries.

## Troubleshooting

- `skillnet hook ingest` is designed for Claude Code hook execution. Without
  `--strict`, database and payload errors are printed to stderr and the command
  exits 0 so a broken logging path does not break interactive tool use.
- Use `skillnet hook ingest --strict --event PostToolUse` in tests and CI when
  failures should be fatal.
- If `skillnet hook install` reports a JSON parse error, clean up comments or
  invalid JSON in the settings file first. The command refuses to rewrite
  invalid JSON rather than risk dropping user content.
- If `skillnet hook status` prints `not installed`, run `skillnet hook install`
  against the same `--settings` path that Claude Code reads.
