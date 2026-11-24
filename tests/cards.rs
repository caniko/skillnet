use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{json, Value};
use tempfile::TempDir;

fn skillnet(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("skillnet").unwrap();
    cmd.current_dir(root)
        .env_remove("AI_SKILLS_REPO")
        .env_remove("CLAUDE_SESSION_ID")
        .env("SKILLNET_CONFIG", root.join("skillnet.toml"))
        .env("skillnet_DATA_DIR", root.join("data"))
        .env_remove("SKILLNET_DATABASE_URL")
        .env_remove("SKILLNET_DB_URL")
        .env_remove("DATABASE_URL");
    cmd
}

fn json_output(command: &mut Command) -> Value {
    serde_json::from_slice(&command.assert().success().get_output().stdout).unwrap()
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn fingerprint_is_deterministic_and_finds_the_repository_root_from_a_subdirectory() {
    let root = TempDir::new().unwrap();
    write(root.path(), "Cargo.toml", "[workspace]\n");
    write(root.path(), "src/nested/lib.rs", "// one\n// two");
    write(root.path(), "flake.nix", "{}");
    write(root.path(), "pyproject.toml", "");
    write(root.path(), "tsconfig.json", "{}");
    write(root.path(), "a/b/design.fig", "");
    write(root.path(), "a/b/c/too-deep.glb", "");
    write(
        root.path(),
        ".git/config",
        "[remote \"origin\"]\nurl = git@codeberg.org:org/repo.git\n",
    );
    let first =
        json_output(skillnet(root.path()).args(["fingerprint", "--cwd", "src/nested", "--json"]));
    let second = json_output(skillnet(root.path()).args(["fingerprint", "--json"]));
    assert_eq!(first, second);
    assert_eq!(first["root"], root.path().to_str().unwrap());
    assert_eq!(
        first["languages"],
        json!(["rust", "nix", "python", "typescript"])
    );
    assert_eq!(first["rust_files"], 1);
    assert_eq!(first["rust_loc"], 2);
    assert_eq!(first["rust_has_workspace"], true);
    assert_eq!(first["forge"], "codeberg");
    assert_eq!(first["has_openpencil"], true);
    assert_eq!(first["has_3d_assets"], false);
    assert!(first["projected_skills"]
        .as_array()
        .unwrap()
        .contains(&json!("openpencil-ui-parity")));
}

const ADVISE: &str = r#"+++
id = "rust/check"
pack = "rust"
tier = "advise"
cooldown = "session"
events = ["post-edit"]
full_ref = "../guide.md"
fixture = "fixtures/positive.json"
negative_fixture = "fixtures/negative.json"
[on]
path = ["src/**/*.rs"]
not_path = ["src/generated/**"]
diagnostic = ["clippy::*"]
diff_regex = ['\bunwrap\(']
+++
Use error handling.
"#;

fn pack(root: &Path) {
    write(
        root,
        "skillnet.toml",
        &format!(
            "[global]\ncanonical_path = {:?}\nviews = []",
            root.join("global_skills").to_str().unwrap()
        ),
    );
    write(
        root,
        "packs/rust/pack.toml",
        "name = \"rust\"\ndescription = \"Rust\"\nlanguages = [\"rust\"]",
    );
    write(root, "packs/rust/cards/check.md", ADVISE);
    write(root, "packs/rust/guide.md", "# Full guide\n");
    write(
        root,
        "packs/rust/fixtures/positive.json",
        r#"{"event":"post-edit","file":"src/lib.rs","new_string":"unwrap("}"#,
    );
    write(
        root,
        "packs/rust/fixtures/negative.json",
        r#"{"event":"post-edit","payload":{"file_path":"src/generated/lib.rs","diff":"unwrap("}}"#,
    );
}

fn trigger(root: &Path, session: &str, event: &str) -> Command {
    let mut command = skillnet(root);
    command.args([
        "trigger",
        "--event",
        event,
        "--state-dir",
        "state",
        "--session",
        session,
        "--json",
    ]);
    command
}

#[test]
fn validates_fixtures_and_triggers_only_for_new_eligible_findings() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    skillnet(root.path())
        .args(["card", "validate"])
        .assert()
        .success();
    let listed =
        json_output(skillnet(root.path()).args(["card", "list", "--pack", "rust", "--json"]));
    assert_eq!(listed[0]["id"], "rust/check");
    let finding = |file: &str| json!({"v":1,"source":"clippy","code":"clippy::x","file":file,"line":4,"severity":"warning"});
    let old = finding("src/nested/lib.rs");
    let mut shifted = old.clone();
    shifted["line"] = json!(5);
    let payload =
        json!({"findings":[shifted, finding("src/generated/lib.rs")], "baseline_findings":[old]});
    let output =
        json_output(trigger(root.path(), "one", "post-edit").write_stdin(payload.to_string()));
    assert_eq!(output["matched_count"], 0);
    let payload = json!({"findings":[shifted, old], "baseline_findings":[old]});
    let output =
        json_output(trigger(root.path(), "one", "post-edit").write_stdin(payload.to_string()));
    assert_eq!(output["inject"][0]["card"], "rust/check");
    let repeat =
        json_output(trigger(root.path(), "one", "post-edit").write_stdin(payload.to_string()));
    assert_eq!(repeat["inject"], json!([]));
    assert_eq!(repeat["collapsed"], json!({"rust/check": 1}));
    write(
        root.path(),
        "packs/rust/fixtures/negative.json",
        r#"{"event":"post-edit","file":"src/lib.rs","new_string":"unwrap("}"#,
    );
    skillnet(root.path())
        .args(["card", "validate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rust/check").and(predicate::str::contains("negative")));
}

#[test]
fn gates_ignore_budgets_and_card_show_records_without_changing_stdout() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    write(
        root.path(),
        "packs/rust/cards/gate.md",
        r#"+++
id = "rust/gate"
pack = "rust"
tier = "gate"
cooldown = "session"
events = ["pre-tool"]
fixture = "fixtures/gate.json"
[on]
tool = ["Bash"]
command_regex = ["push"]
not_command_regex = ["--dry-run"]
repo_expr = 'has_rust && (rust_loc >= 10 || forge == "github")'
+++
Stop and check.
"#,
    );
    let tool = json!({"tool_name":"bash","tool_input":{"command":"git push"}});
    write(
        root.path(),
        "packs/rust/fixtures/gate.json",
        &json!({"event":"pre-tool","payload":tool}).to_string(),
    );
    skillnet(root.path())
        .args(["card", "validate"])
        .assert()
        .success();
    for _ in 0..2 {
        let output = json_output(
            trigger(root.path(), "one", "pre-tool")
                .args(["--max-cards", "0", "--max-tokens", "0"])
                .write_stdin(tool.to_string()),
        );
        assert_eq!(
            output["deny"],
            json!({"card":"rust/gate","reason":"Stop and check."})
        );
    }
    let payload = json!({"tool_name":"Bash", "tool_input":{"command":"git push --dry-run"},
        "fingerprint":{"has_rust":true,"rust_loc":9,"forge":"github"}});
    let output =
        json_output(trigger(root.path(), "one", "pre-tool").write_stdin(payload.to_string()));
    assert_eq!(output["deny"]["card"], "rust/gate"); // independent repo-expression family
    let payload = json!({"tool_name":"Bash", "tool_input":{"command":"git push --dry-run"}});
    let output =
        json_output(trigger(root.path(), "one", "pre-tool").write_stdin(payload.to_string()));
    assert_eq!(output["deny"], Value::Null);
    skillnet(root.path())
        .args(["card", "show", "rust/check", "--full", "--no-record"])
        .assert()
        .success()
        .stdout("# Full guide\n");
    let db = root.path().join("data/multi-phase-plan/calibration.sqlite");
    assert!(!db.exists());
    skillnet(root.path())
        .args([
            "card",
            "show",
            "rust/check",
            "--harness",
            "codex",
            "--session",
            "one",
        ])
        .assert()
        .success()
        .stdout("Use error handling.\n");
    let conn = rusqlite::Connection::open(db).unwrap();
    let recorded: i64 = conn
        .query_row(
            "SELECT count(*) FROM skill_invocations WHERE session_id = 'one' AND harness = 'codex'
         AND canonical_skill_name = 'rust/check' AND hook_event = 'CardRead'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recorded, 1);
}

#[test]
fn trigger_persists_budgets_and_repo_cooldowns_but_not_budget_rejections() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    for (name, tier, cooldown) in [
        ("a-repo", "advise", "repo-30d"),
        ("b-always", "advise", "always"),
        ("c-silent", "fix", "always"),
    ] {
        write(root.path(), &format!("packs/rust/cards/{name}.md"), &format!(
            "+++\nid = \"rust/{name}\"\npack = \"rust\"\ntier = \"{tier}\"\ncooldown = \"{cooldown}\"\n\
             fixture = \"fixtures/ci.json\"\nevents = [\"ci\"]\n[on]\nsignal = [\"oom_kill\"]\n+++\nNote\n"
        ));
    }
    write(
        root.path(),
        "packs/rust/fixtures/ci.json",
        r#"{"event":"ci","signals":["oom_kill"]}"#,
    );
    write(root.path(), "payload.json", r#"{"signal":"oom_kill"}"#);
    skillnet(root.path())
        .args(["card", "validate"])
        .assert()
        .success();
    let run = |session: &str, tokens: &str| {
        json_output(trigger(root.path(), session, "ci").args([
            "--payload-file",
            "payload.json",
            "--max-cards",
            "1",
            "--max-tokens",
            tokens,
        ]))
    };
    let first = run("one", "1");
    assert_eq!(first["matched_count"], 3);
    assert_eq!(first["inject"].as_array().unwrap().len(), 1);
    assert_eq!(first["inject"][0]["card"], "rust/a-repo");
    assert_eq!(first["budget_dropped"], 1);
    let second = run("one", "1");
    assert_eq!(second["inject"], json!([])); // session budget already spent
    assert_eq!(second["collapsed"], json!({"rust/a-repo":1}));
    assert_eq!(second["budget_dropped"], 1);
    let new_session = run("two", "2");
    assert_eq!(new_session["inject"][0]["card"], "rust/b-always"); // repo cooldown crosses sessions
    assert_eq!(run("two", "2")["inject"][0]["card"], "rust/b-always"); // always can repeat
    assert_eq!(run("two", "2")["inject"], json!([]));
}

#[test]
fn trigger_handles_raw_claude_edits_and_mcp_tool_names() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    for (path_key, diff_key) in [
        ("file_path", "new_string"),
        ("path", "content"),
        ("path", "old_string"),
    ] {
        let payload = json!({
            "tool_name": "Edit",
            "tool_input": { (path_key): root.path().join("src/lib.rs"), (diff_key): "unwrap(" },
        });
        let output = json_output(
            trigger(root.path(), diff_key, "post-edit").write_stdin(payload.to_string()),
        );
        assert_eq!(output["inject"][0]["card"], "rust/check");
    }
    write(
        root.path(),
        "packs/rust/cards/merge.md",
        r#"+++
id = "rust/merge"
pack = "rust"
tier = "gate"
cooldown = "always"
events = ["pre-tool"]
fixture = "fixtures/merge.json"
[on]
tool = ["pull_request_merge"]
+++
Check the merge.
"#,
    );
    let payload = json!({"tool_name":"mcp__t3-code__pull_request_merge","tool_input":{"number":3}});
    let output =
        json_output(trigger(root.path(), "one", "pre-tool").write_stdin(payload.to_string()));
    assert_eq!(output["deny"]["card"], "rust/merge");
}

#[test]
fn audit_sweep_uses_recommended_packs_and_honors_explicit_budgets() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    write(root.path(), "Cargo.toml", "[workspace]\n");
    write(
        root.path(),
        "packs/nix/pack.toml",
        "name = \"nix\"\ndescription = \"Nix\"",
    );
    for (pack, name, tier, event, expr) in [
        ("rust", "a", "advise", "audit", "has_rust"),
        ("rust", "b", "advise", "audit", "has_rust"),
        ("rust", "c", "advise", "audit", ""),
        ("rust", "false-expr", "advise", "audit", "!has_rust"),
        ("rust", "not-audit", "advise", "ci", "has_rust"),
        ("rust", "gate", "gate", "audit", "has_rust"),
        ("nix", "unrecommended", "advise", "audit", "has_rust"),
    ] {
        write(root.path(), &format!("packs/{pack}/cards/{name}.md"), &format!(
            "+++\nid = \"{pack}/{name}\"\npack = \"{pack}\"\ntier = \"{tier}\"\ncooldown = \"always\"\n\
             fixture = \"unused.json\"\nevents = [\"{event}\"]\n[on]\npath = [\"src/**/*.rs\"]\n\
             repo_expr = \"{expr}\"\n+++\nNote\n"
        ));
    }
    let sweep = json_output(trigger(root.path(), "default", "audit").write_stdin(""));
    assert_eq!(sweep["matched_count"], 3);
    assert_eq!(sweep["inject"].as_array().unwrap().len(), 3);
    assert!(sweep["deny"].is_null());
    let capped = json_output(
        trigger(root.path(), "explicit", "audit")
            .args(["--no-stdin", "--max-cards", "2"])
            .write_stdin("not JSON"),
    );
    assert_eq!(capped["inject"].as_array().unwrap().len(), 2);
    assert_eq!(capped["budget_dropped"], 1);
    let scoped =
        json_output(trigger(root.path(), "scoped", "audit").write_stdin(r#"{"file":"README.md"}"#));
    assert_eq!(scoped["matched_count"], 0);
}

#[test]
fn corrupt_session_state_is_replaced() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    let run = || {
        trigger(root.path(), "one", "post-edit")
            .write_stdin(r#"{"file":"src/lib.rs","new_string":"unwrap("}"#)
            .assert()
            .success()
    };
    run();
    let state = fs::read_dir(root.path().join("state"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    fs::write(&state, "{").unwrap();
    run()
        .stderr(predicate::str::contains("warning:"))
        .stdout(predicate::str::contains("Use error handling."));
    let state: Value = serde_json::from_slice(&fs::read(state).unwrap()).unwrap();
    assert_eq!(state["cards"], json!(["rust/check"]));
}

#[test]
fn full_ref_cannot_escape_the_packs_parent() {
    let outside = TempDir::new().unwrap();
    let root = outside.path().join("source");
    pack(&root);
    write(outside.path(), "outside.md", "Outside the skills source.\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("outside.md"),
        root.join("packs/rust/link.md"),
    )
    .unwrap();
    for reference in [
        "../../../../outside.md",
        #[cfg(unix)]
        "../link.md",
    ] {
        write(
            &root,
            "packs/rust/cards/check.md",
            &ADVISE.replace("../guide.md", reference),
        );
        for args in [
            &["card", "validate"][..],
            &["card", "show", "rust/check", "--full", "--no-record"][..],
        ] {
            skillnet(&root)
                .args(args)
                .assert()
                .failure()
                .stdout(predicate::str::is_empty())
                .stderr(predicate::str::contains(
                    "full_ref must resolve to a file inside",
                ));
        }
    }
}

#[test]
fn card_show_warns_but_succeeds_when_recording_fails() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    for no_record in [false, true] {
        let mut cmd = skillnet(root.path());
        cmd.args([
            "--database-url",
            "not://a/postgres/url",
            "card",
            "show",
            "rust/check",
        ]);
        if no_record {
            cmd.arg("--no-record");
        }
        let output = cmd.assert().success().stdout("Use error handling.\n");
        if no_record {
            output.stderr(predicate::str::is_empty());
        } else {
            output.stderr(predicate::str::contains(
                "skillnet: warning: failed to record card read:",
            ));
        }
    }
}

#[test]
fn resolves_pack_paths_in_precedence_order() {
    let root = TempDir::new().unwrap();
    let global = TempDir::new().unwrap();
    pack(global.path());
    write(
        root.path(),
        "skillnet.toml",
        &format!(
            "[global]\ncanonical_path = {:?}\nviews = []",
            global.path().join("global_skills").to_str().unwrap()
        ),
    );
    assert_eq!(
        json_output(skillnet(root.path()).args(["card", "list", "--json"]))[0]["id"],
        "rust/check"
    );
    let empty = TempDir::new().unwrap();
    fs::create_dir(empty.path().join("packs")).unwrap();
    assert_eq!(
        json_output(
            skillnet(root.path())
                .env("AI_SKILLS_REPO", empty.path())
                .args(["card", "list", "--json"])
        ),
        json!([])
    );
    fs::create_dir(root.path().join("packs")).unwrap();
    assert_eq!(
        json_output(skillnet(root.path()).args(["card", "list", "--json"]))[0]["id"],
        "rust/check"
    );
    fs::remove_file(root.path().join("skillnet.toml")).unwrap();
    skillnet(root.path())
        .args(["card", "list", "--json"])
        .assert()
        .failure();
    write(
        root.path(),
        "skillnet.toml",
        "[global]\ncanonical_path = \"global_skills\"\nviews = []",
    );
    skillnet(root.path())
        .env_remove("SKILLNET_CONFIG")
        .env("XDG_CONFIG_HOME", root.path().join("empty-xdg"))
        .args(["card", "list", "--json"])
        .assert()
        .failure();
    assert_eq!(
        json_output(
            skillnet(root.path())
                .env("AI_SKILLS_REPO", empty.path())
                .args(["card", "list", "--json", "--packs-dir"])
                .arg(global.path().join("packs"))
        )[0]["id"],
        "rust/check"
    );
}
