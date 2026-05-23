# Migration: skillnet CLI rebuild

## TL;DR

The CLI tree was reorganized around what you're acting on: `sync`, `skill`, `scope`, `project`, `catalog`. Every old verb maps to one new verb. No hidden aliases - old invocations error.

## Removed top-level shortcuts

| Old | New |
|---|---|
| `skillnet reconcile` | `skillnet sync pull` |
| `skillnet reconcile --sync` | `skillnet sync pull --then-push` |
| `skillnet sync` | `skillnet sync push` |
| `skillnet delete <s> <k>` | `skillnet skill delete <s>/<k>` |
| `skillnet rename <s> <o> <n>` | `skillnet skill rename <s>/<o> <n>` |
| `skillnet move <fs> <k> <ts>` | `skillnet skill move <fs>/<k> <ts>` |
| `skillnet globalize <p> <k>` | `skillnet skill move <p>/<k> global` |
| `skillnet deglobalize <k> <p>` | `skillnet skill move global/<k> <p>` |
| `skillnet list` | `skillnet skill list` |
| `skillnet targets` | `skillnet scope list` |
| `skillnet sources --target X` | `skillnet scope sources --scope X` |
| `skillnet project ...` | unchanged |

## Removed namespaces

- `skillnet mirror <verb>` - every verb moved under `sync`, `skill`, or `scope` as above.
- `skillnet toml project <verb>` - moved to top-level `skillnet project <verb>`.
- `skillnet catalog show <skill>` - folded into `skillnet skill show <scope>/<skill>`.

## Flag changes

- `--sync` on edit verbs: **removed**. Run `skillnet sync push --scope <scope>` afterward.
- `--target <all|global|project|<name>>`: **removed**. Use `--scope` (repeatable) and `--all`.
- Per-command `--dry-run`: **removed**. Use the global `--dry-run` flag: `skillnet --dry-run sync push`.

## New commands

- `skillnet` (no args): runs `status`.
- `skillnet status`: scopes + divergence + catalog health.
- `skillnet sync status`: read-only divergence per scope.
- `skillnet sync diff`: file-level diff mirror<->live.
- `skillnet sync pull --then-push`: composed pull then push.

## Caching

`mirror_root/.skillnet/cache.toml` stores per-scope pull timestamps and content hashes. `status` and `sync status` use it to skip redundant walks. The cache is best-effort; deleting or corrupting it falls back to a full walk on the next command.

## Common workflows

Edit and push:

```sh
skillnet skill move global/foo myproj
skillnet sync push --scope global --scope myproj
```

Refresh from live and immediately re-mirror:

```sh
skillnet sync pull --then-push
```

Check what would change without writing:

```sh
skillnet sync status
skillnet sync diff
skillnet --dry-run sync push
```
