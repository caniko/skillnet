# Command Surface

Top-level commands:

- `status`: show scope divergence and catalog health.
- `completions`: generate shell completion scripts.
- `sync`: pull from live sources, push to live targets, and inspect divergence.
- `skill`: list, inspect, and edit mirrored skill directories.
- `scope`: inspect configured mirror scopes and their live sources.
- `project`: manage configured project roots.
- `catalog`: generate and validate skill catalog metadata.
- `calibration`: record and analyze `multi-phase-plan` calibration data.

Examples:

```sh
skillnet sync pull --scope global
skillnet sync status --scope global
skillnet skill show global/rust-project-flake
skillnet project list
skillnet catalog lint
```
