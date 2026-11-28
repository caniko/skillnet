"""Build, validate, and sync Skillnet's harness integration manifest."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import tempfile


HARNESS = ("opencode", "antigravity", "agents", "claude")
AGENTS = ("antigravity", "agents")
NAME = r"[a-z0-9][a-z0-9_-]*"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def safe_text(text):
    return isinstance(text, str) and bool(text) and not re.search(r"[\\\x00-\x1f\x7f]", text)


def clean_path(path):
    return safe_text(path) and not set(path.split("/")) & {"", ".", ".."}


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def json_bytes(data):
    return (json.dumps(data, indent=2, ensure_ascii=False) + "\n").encode()


def read_json(path):
    return json.loads(path.read_bytes())


def inventory(manifest):
    """Validate ownership data before using any path from either manifest."""
    require(isinstance(manifest, dict) and type(manifest.get("schemaVersion")) is int
            and manifest["schemaVersion"] == 1, "expected integration manifest schemaVersion 1")
    require(isinstance(manifest.get("name"), str) and manifest["name"], "missing integration name")
    managed = manifest.get("managedManifest")
    require(clean_path(managed), "managedManifest must be a clean relative path")
    harnesses = manifest.get("harnesses")
    require(isinstance(harnesses, list) and all(h in HARNESS for h in harnesses), "invalid harness list")
    paths = {}
    for kind, field in (("files", "sha256"), ("symlinks", "target")):
        require(isinstance(manifest.get(kind), list), f"missing manifest {kind} list")
        for entry in manifest[kind]:
            require(isinstance(entry, dict) and set(entry) == {"path", "harness", field}, f"invalid {kind} entry")
            path, harness, value = entry["path"], entry["harness"], entry[field]
            require(clean_path(path) and path not in paths, f"unsafe or duplicate path: {path!r}")
            require(harness in harnesses, f"{path}: harness is not declared")
            if kind == "symlinks":
                require(harness in AGENTS and re.fullmatch(rf"\.agents/skills/{NAME}", path),
                        f"{path}: invalid skill symlink path")
                require(safe_text(value) and not value.startswith("/"), f"{path}: expected a relative target")
            else:
                pattern = rf"\.agents/skills/{NAME}/.+" if harness in AGENTS else rf"\.{harness}/commands/{NAME}\.md"
                require(re.fullmatch(pattern, path), f"{path}: invalid {harness} file path")
                require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value), f"{path}: invalid SHA-256")
            paths[path] = entry
    leaves = paths.keys() | {managed}
    require(managed not in paths, f"{managed}: managedManifest overlaps a generated entry")
    for path in leaves:
        require(not any(parent.as_posix() in leaves for parent in Path(path).parents),
                f"{path}: a manifest entry is also a parent of this path")
    return paths


def regular_files(root):
    for directory, subdirs, files in os.walk(root):
        for item in subdirs + files:
            path = Path(directory, item)
            expected = stat.S_ISDIR if item in subdirs else stat.S_ISREG
            require(expected(path.lstat().st_mode), f"{path}: expected a regular file or directory (not a symlink)")
        yield from (Path(directory, file) for file in files)


def gitignore_path(path):
    return re.sub(r"([*?\[\] ])", r"\\\1", str(path))


def build(config, out):
    root = out / "files"
    root.mkdir(parents=True)
    manifest = {key: config[key] for key in ("name", "harnesses", "managedManifest")}
    manifest.update(schemaVersion=1, files=[], symlinks=[])
    rules = set()
    for entry in config["entries"] + config["skillEntries"]:
        path, harness = entry["path"], entry["harness"]
        dest = root / path
        if "target" in entry:
            manifest["symlinks"].append(dict(path=path, harness=harness, target=entry["target"]))
            relatives = [path]
        else:
            if "source" in entry:
                source = Path(entry["source"])
                require((source / "SKILL.md").is_file(), f"{path}: source must contain SKILL.md")
                shutil.copytree(source, dest, symlinks=True)
                files = list(regular_files(dest))
            else:
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(entry["text"].encode())
                files = [dest]
            relatives = [file.relative_to(root).as_posix() for file in files]
            manifest["files"].extend(dict(path=file, harness=harness, sha256=sha256(root / file)) for file in relatives)
        for relative in relatives:
            for parent in Path(relative).parents:
                if parent != Path("."):
                    rules.add(f"!/{gitignore_path(parent)}/")
            rules.add(f"!/{gitignore_path(relative)}" if harness in AGENTS else f"!/.{harness}/commands/*.md")
    for kind in ("files", "symlinks"):
        manifest[kind].sort(key=lambda entry: entry["path"])
    inventory(manifest)
    (out / "manifest.json").write_bytes(json_bytes(manifest))
    (out / "gitignore.snippet").write_text("".join(f"{rule}\n" for rule in sorted(rules)), encoding="utf-8")


def check_artifact(integration, manifest):
    import yaml

    root = integration / "files"
    require(stat.S_ISDIR(mode(root)), "integration/files must be a directory")
    regular = {entry["path"]: entry for entry in manifest["files"]}
    require(set(regular) == {path.relative_to(root).as_posix() for path in regular_files(root)},
            "manifest files do not match integration/files")
    for relative, entry in regular.items():
        path = root / relative
        require(sha256(path) == entry["sha256"], f"{relative}: SHA-256 mismatch")
        if entry["harness"] in AGENTS:
            skill = "/".join(Path(relative).parts[:3]) + "/SKILL.md"
            require(skill in regular, f"{relative}: skill is missing SKILL.md")
            if relative != skill:
                continue
        lines = path.read_text(encoding="utf-8").split("\n")
        if entry["harness"] == "claude":
            require(lines and lines[0] == f"# /{path.stem}", f"{relative}: invalid Claude heading")
            continue
        require(lines and lines[0] == "---" and "---" in lines[1:], f"{relative}: missing YAML frontmatter")
        try:
            metadata = yaml.load("\n".join(lines[1:lines.index("---", 1)]), Loader=yaml.BaseLoader)
        except yaml.YAMLError as error:
            raise ValueError(f"{relative}: invalid YAML frontmatter: {error}") from error
        description = metadata.get("description") if isinstance(metadata, dict) else None
        require(isinstance(description, str) and description.strip(), f"{relative}: missing description")
        if entry["harness"] in AGENTS:
            require(metadata.get("name") == path.parent.name, f"{relative}: name must match the skill directory")


def mode(path):
    try:
        return path.lstat().st_mode
    except FileNotFoundError:
        return 0


def workspace_mode(root, relative, removed):
    path = Path(relative)
    for parent in reversed((path, *path.parents)[:-1]):
        if parent.as_posix() in removed:
            return 0
        current = mode(root / parent)
        if not current or parent == path:
            return current
        require(stat.S_ISDIR(current), f"{parent}: workspace parent is not a directory (refusing to follow it)")
    return 0


def matches(path, current, entry, source):
    if "target" in entry:
        return stat.S_ISLNK(current) and os.readlink(path) == entry["target"]
    return stat.S_ISREG(current) and path.read_bytes() == source.read_bytes()


def atomic_write(path, data):
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temp:
        staged = Path(temp.name)
        try:
            temp.write(data)
            temp.close()
            staged.chmod(0o644)
            staged.replace(path)
        finally:
            staged.unlink(missing_ok=True)


def sync(integration, manifest, paths, root, check, force):
    require(root.is_dir(), f"{root}: workspace root is not a directory")
    managed = manifest["managedManifest"]
    managed_path = root / managed
    managed_mode = workspace_mode(root, managed, set())
    previous = {}
    if managed_mode:
        require(stat.S_ISREG(managed_mode), f"{managed}: expected a regular manifest file")
        old = read_json(managed_path)
        previous = inventory(old)
        require(old["managedManifest"] == managed, f"{managed}: previous manifest names a different path")

    stale = sorted(previous.keys() - paths.keys())
    removed = {}
    prune = set()
    messages = []
    for relative in stale:
        current = workspace_mode(root, relative, removed)
        if current:
            messages.append(f"{relative}: stale managed entry")
            if not check:
                require(not stat.S_ISDIR(current), f"{relative}: stale managed entry is now a directory; remove it manually")
                removed[relative] = current
        # Only remove empty structural directories inside previously managed skills.
        prune.update(p.as_posix() for p in Path(relative).parents if p.as_posix().startswith(".agents/skills/"))
    if not check:
        for relative in sorted(prune, key=lambda path: (-path.count("/"), path)):
            current = workspace_mode(root, relative, removed)
            if stat.S_ISDIR(current) and all(f"{relative}/{child.name}" in removed for child in (root / relative).iterdir()):
                removed[relative] = current
    writes = []
    for relative, entry in paths.items():
        current = workspace_mode(root, relative, removed)
        path = root / relative
        if matches(path, current, entry, integration / "files" / relative):
            continue
        messages.append(f"{relative}: {'differs' if current else 'missing'}")
        if check:
            continue
        if current:
            require(force or relative in previous or ("target" in entry and stat.S_ISLNK(current)),
                    f"{relative}: refusing to overwrite an unmanaged path (use --force)")
            if stat.S_ISDIR(current):
                require("target" in entry and not any(path.iterdir()),
                        f"{relative}: refusing to replace a directory with unmanaged contents")
                removed[relative] = current
        writes.append((relative, entry))
    data = (integration / "manifest.json").read_bytes()
    manifest_changed = not managed_mode or managed_path.read_bytes() != data
    if check:
        if manifest_changed:
            messages.append(f"{managed}: managed manifest is missing or differs")
        for message in messages:
            print(message, file=sys.stderr)
        return bool(messages)

    # ponytail: a single writer owns the workspace during sync; use dirfd-based
    # operations if simultaneous untrusted workspace mutation must be supported.
    for relative, current in removed.items():
        path = root / relative
        path.rmdir() if stat.S_ISDIR(current) else path.unlink()
    for relative, entry in writes:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if "target" in entry:
            with tempfile.TemporaryDirectory(dir=path.parent) as directory:
                link = Path(directory, "link")
                link.symlink_to(entry["target"])
                link.replace(path)
        else:
            atomic_write(path, (integration / "files" / relative).read_bytes())
    if manifest_changed:
        managed_path.parent.mkdir(parents=True, exist_ok=True)
        atomic_write(managed_path, data)
    return 0


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    builder = commands.add_parser("build")
    builder.add_argument("config", type=Path)
    builder.add_argument("out", type=Path)
    commands.add_parser("check").add_argument("integration", type=Path)
    syncer = commands.add_parser("sync")
    syncer.add_argument("integration", type=Path, help=argparse.SUPPRESS)
    syncer.add_argument("--root", type=Path, default=Path.cwd())
    flags = syncer.add_mutually_exclusive_group()
    flags.add_argument("--check", action="store_true")
    flags.add_argument("--force", action="store_true")
    flags.add_argument("--print-gitignore", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(read_json(args.config), args.out)
            return 0
        if args.command == "sync" and args.print_gitignore:
            sys.stdout.buffer.write((args.integration / "gitignore.snippet").read_bytes())
            return 0
        manifest = read_json(args.integration / "manifest.json")
        paths = inventory(manifest)
        if args.command == "sync":
            return sync(args.integration, manifest, paths, args.root, args.check, args.force)
        check_artifact(args.integration, manifest)
        return 0
    except (OSError, ValueError) as error:
        print(f"skillnet integrations: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
