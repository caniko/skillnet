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

#[test]
fn fingerprint_counts_workspace_sources_without_counting_builds_or_test_crates() {
    let root = TempDir::new().unwrap();
    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nmembers = ['crates/a', 'crates/b']",
    );
    write(root.path(), "crates/a/src/lib.rs", "// one\n// two");
    write(root.path(), "crates/b/src/lib.rs", "// three\n");
    write(root.path(), ".git/copied/src/lib.rs", "// excluded");
    for dir in [
        "target",
        ".hidden",
        ".direnv",
        "node_modules",
        "vendor",
        "dist",
        "build",
        "result",
        "tests",
        "benches",
        "examples",
    ] {
        write(
            root.path(),
            &format!("crates/a/{dir}/copied/src/lib.rs"),
            "// excluded",
        );
    }
    write(root.path(), "crates/b/build.rs", "// not in src");
    write(root.path(), "a/b/c/d/e/f/g/src/lib.rs", "// too deep");
    let fingerprint = || json_output(skillnet(root.path()).args(["fingerprint", "--json"]));
    let virtual_workspace = fingerprint();
    assert_eq!(virtual_workspace["rust_has_workspace"], true);
    assert_eq!(virtual_workspace["rust_files"], 2);
    assert_eq!(virtual_workspace["rust_loc"], 3);
    write(root.path(), "src/lib.rs", "// root crate");
    assert_eq!(fingerprint()["rust_files"], 3);
    write(root.path(), "Cargo.toml", "[package]\nname = 'standalone'");
    assert_eq!(fingerprint()["rust_files"], 1);
    assert_eq!(fingerprint()["rust_loc"], 1);
    fs::remove_file(root.path().join("src/lib.rs")).unwrap();
    fs::remove_dir(root.path().join("src")).unwrap();
    assert_eq!(fingerprint()["rust_files"], 0);
}

#[test]
fn fingerprint_skips_nested_repositories_and_symlinks() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    write(root.path(), "checkout/.git/config", "");
    write(root.path(), "worktree/.git", "gitdir: elsewhere\n");
    for dir in ["checkout", "worktree", "vendor", ".hidden"] {
        write(root.path(), &format!("{dir}/src/lib.rs"), "// excluded");
        write(root.path(), &format!("{dir}/design.fig"), "");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("checkout/src", root.path().join("src")).unwrap();
        std::os::unix::fs::symlink("checkout/design.fig", root.path().join("linked.fig")).unwrap();
    }
    let fingerprint = || json_output(skillnet(root.path()).args(["fingerprint", "--json"]));
    let without_cargo = fingerprint();
    assert_eq!(without_cargo["has_rust"], false);
    assert_eq!(without_cargo["has_openpencil"], false);
    assert_eq!(without_cargo["rust_files"], 0);
    assert_eq!(without_cargo["rust_loc"], 0);
    write(root.path(), "Cargo.toml", "[package]\nname = 'standalone'");
    assert_eq!(fingerprint()["rust_files"], 0);
    #[cfg(unix)]
    fs::remove_file(root.path().join("src")).unwrap();
    write(root.path(), "src/nested/.git", "gitdir: elsewhere\n");
    write(root.path(), "src/nested/lib.rs", "// excluded");
    write(root.path(), "src/lib.rs", "// included");
    assert_eq!(fingerprint()["rust_files"], 1);
    write(root.path(), "Cargo.toml", "[workspace]");
    write(root.path(), "crates/a/src/lib.rs", "// included");
    let workspace = fingerprint();
    assert_eq!(workspace["rust_files"], 2);
    assert_eq!(workspace["rust_loc"], 2);
}

#[test]
fn fingerprint_uses_the_nearest_cargo_root_or_a_shallow_workspace() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    write(root.path(), "cli/Cargo.toml", "[workspace]");
    write(root.path(), "cli/crates/a/src/lib.rs", "// one\n// two");
    write(root.path(), "cli/crates/b/src/lib.rs", "// three\n");
    write(root.path(), "outside/src/lib.rs", "// not in CLI");
    let fingerprint =
        |cwd| json_output(skillnet(root.path()).args(["fingerprint", "--cwd", cwd, "--json"]));
    for cwd in ["cli", "cli/crates/a/src", "."] {
        let output = fingerprint(cwd);
        assert_eq!(output["root"], root.path().to_str().unwrap());
        assert_eq!(output["rust_has_workspace"], true, "{cwd}");
        assert_eq!(output["rust_files"], 2, "{cwd}");
        assert_eq!(output["rust_loc"], 3, "{cwd}");
    }
    write(
        root.path(),
        "cli/crates/a/Cargo.toml",
        "[package]\nname = 'a'",
    );
    let member = fingerprint("cli/crates/a/src");
    assert_eq!(member["rust_has_workspace"], false);
    assert_eq!(member["rust_files"], 1);
    assert_eq!(member["rust_loc"], 2);
}

fn finding(source: &str, code: &str, file: Option<&str>, line: u64, severity: &str) -> Value {
    json!({"v":1,"source":source,"code":code,"file":file,"line":line,"severity":severity})
}

#[test]
fn findings_reads_cargo_json_and_selects_primary_span_or_first_span() {
    let root = TempDir::new().unwrap();
    let diagnostic = |code, level, primary| {
        json!({
            "reason": "compiler-message", "message": {
                "level": level, "code": {"code": code}, "spans": [
                    {"file_name": "src/first.rs", "line_start": 3, "is_primary": false},
                    {"file_name": "src/main.rs", "line_start": 8, "is_primary": primary}
                ]
            }
        })
    };
    let input = format!(
        "Compiling crate\n{}\n{}\n{}\n{}\n{}\n",
        json!({"reason":"build-finished"}),
        diagnostic("E0382", "error", true),
        diagnostic("clippy::too_many_arguments", "warning", false),
        diagnostic("dead_code", "warning", true),
        diagnostic("dead_code", "note", true),
    );
    let expected = json!({"findings": [
        finding("rustc", "rustc::E0382", Some("src/main.rs"), 8, "error"),
        finding("clippy", "clippy::too_many_arguments", Some("src/first.rs"), 3, "warn"),
        finding("rustc", "rustc::dead_code", Some("src/main.rs"), 8, "warn"),
    ]});
    for format in ["cargo-json", "auto"] {
        let output = json_output(
            skillnet(root.path())
                .args(["findings", "--from", format])
                .write_stdin(input.as_str()),
        );
        assert_eq!(output, expected);
    }
}

const CARGO_STDERR: &str = "\
error[E0382]: borrow of moved value
 --> src/main.rs:12:5
 ::: src/other.rs:4:1

error[E0277]: a trait bound is not satisfied
 --> src/main.rs:24:8

warning: this function has too many arguments
 --> src/lib.rs:4:1
  = note: `#[warn(clippy::too_many_arguments)]` on by default

warning: function is never used
 --> src/lib.rs:22:9
  = note: `#[warn(dead_code)]` on by default

error: used `unwrap()`
 --> src/lib.rs:30:1
  = note: requested on the command line with `-D clippy::unwrap-used`
";

#[test]
fn findings_reads_cargo_text_and_opencode_lsp_codes() {
    let root = TempDir::new().unwrap();
    let input = format!(
        "{CARGO_STDERR}\nERROR [12:5] [E0382] moved\n\
        <diagnostics file=\"crates/other crate/src/lib.rs\">\n\
        WARN [4:1] [clippy::unwrap_used] used unwrap\n</diagnostics>"
    );
    let expected = json!({"findings": [
        finding("rustc", "rustc::E0382", Some("src/main.rs"), 12, "error"),
        finding("rustc", "rustc::E0277", Some("src/main.rs"), 24, "error"),
        finding("clippy", "clippy::too_many_arguments", Some("src/lib.rs"), 4, "warn"),
        finding("rustc", "rustc::dead_code", Some("src/lib.rs"), 22, "warn"),
        finding("clippy", "clippy::unwrap_used", Some("src/lib.rs"), 30, "error"),
        finding("rustc", "rustc::E0382", None, 12, "error"),
        finding("clippy", "clippy::unwrap_used", Some("crates/other crate/src/lib.rs"), 4, "warn"),
    ]});
    for format in ["cargo-text", "auto"] {
        assert_eq!(
            json_output(
                skillnet(root.path())
                    .args(["findings", "--from", format])
                    .write_stdin(input.as_str())
            ),
            expected,
        );
    }
}

#[test]
fn findings_reads_both_jscpd_file_reports() {
    let root = TempDir::new().unwrap();
    for (name, report, expected) in [
        (
            "jscpd-report.json",
            json!({"duplicates": [
                {"firstFile": {"name":"src/a.rs","start":3}, "secondFile": {"name":"src/b.rs","start":5}},
                {"firstFile": {"name":"src/c.rs","start":7}, "secondFile": {"name":"src/c.rs","start":7}},
                {"firstFile": {"name":"src/d.rs","start":9}, "secondFile": {"name":"src/d.rs","start":15}}
            ]}),
            json!([
                finding("jscpd", "jscpd::clone", Some("src/a.rs"), 3, "warn"),
                finding("jscpd", "jscpd::clone", Some("src/b.rs"), 5, "warn"),
                finding("jscpd", "jscpd::clone", Some("src/c.rs"), 7, "warn"),
                finding("jscpd", "jscpd::clone", Some("src/d.rs"), 9, "warn"),
                finding("jscpd", "jscpd::clone", Some("src/d.rs"), 15, "warn"),
            ]),
        ),
        (
            "basta-report.json",
            json!({"findings": [
                {"category":"dead_code", "path":"src/lib.rs", "start":{"line":7,"column":2}}
            ]}),
            json!([finding(
                "jscpd",
                "jscpd::dead_code",
                Some("src/lib.rs"),
                7,
                "warn"
            )]),
        ),
    ] {
        write(root.path(), name, &report.to_string());
        for format in ["auto", "jscpd"] {
            let output = skillnet(root.path())
                .args(["findings", "--from", format, "--input", name, "--pretty"])
                .assert()
                .success()
                .get_output()
                .stdout
                .clone();
            assert!(output.starts_with(b"{\n"));
            assert_eq!(
                serde_json::from_slice::<Value>(&output).unwrap(),
                json!({"findings": expected})
            );
        }
    }
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
fn post_tool_extracts_raw_harness_output_but_respects_supplied_findings() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    for (name, diagnostic) in [
        ("ownership-errors", "rustc::E0382"),
        ("trait-errors", "rustc::E0277"),
        ("delete-first", "clippy::too_many_arguments"),
        ("std-traits", "clippy::should_implement_trait"),
    ] {
        write(root.path(), &format!("packs/rust/cards/{name}.md"), &format!(
            "+++\nid = \"rust/{name}\"\npack = \"rust\"\ntier = \"advise\"\ncooldown = \"session\"\n\
             fixture = \"unused.json\"\nevents = [\"post-tool\"]\n[on]\npath = [\"**/src/**/*.rs\"]\n\
             diagnostic = [\"{diagnostic}\"]\n+++\nNote\n"
        ));
    }
    let cargo = json!({"reason":"compiler-message","message":{
        "level":"warning","code":{"code":"clippy::should_implement_trait"},
        "spans":[{"file_name":"src/lib.rs","line_start":7,"is_primary":true}]
    }});
    for (session, payload, expected) in [
        (
            "claude",
            json!({"hook_event_name":"PostToolUse", "tool_name":"Bash",
            "tool_input":{"command":"cargo clippy"},
            "tool_response":{"stderr": CARGO_STDERR}}),
            json!([
                "rust/delete-first",
                "rust/ownership-errors",
                "rust/trait-errors"
            ]),
        ),
        (
            "claude-failure",
            json!({"hook_event_name":"PostToolUseFailure","tool_name":"Bash",
            "error":"error[E0382]: moved\n --> src/main.rs:12:5"}),
            json!(["rust/ownership-errors"]),
        ),
        (
            "opencode",
            json!({"tool":"bash","result":{"output":format!("{cargo}\n{CARGO_STDERR}")}}),
            json!([
                "rust/delete-first",
                "rust/ownership-errors",
                "rust/std-traits",
                "rust/trait-errors"
            ]),
        ),
        (
            "supplied",
            json!({"stderr":CARGO_STDERR,
            "findings":[{"code":"rustc::E0277", "file":"src/main.rs"}]}),
            json!(["rust/trait-errors"]),
        ),
    ] {
        let output = json_output(
            trigger(root.path(), session, "post-tool")
                .args(["--max-cards", "10"])
                .write_stdin(payload.to_string()),
        );
        assert_eq!(output["event"], "post-tool");
        let ids: Vec<_> = output["inject"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| &card["card"])
            .collect();
        assert_eq!(json!(ids), expected);
    }
}

#[test]
fn pre_push_base_shares_state_and_budget_across_files_and_stdin_findings() {
    let root = TempDir::new().unwrap();
    pack(root.path());
    write(
        root.path(),
        "packs/rust/cards/check.md",
        &ADVISE.replace("post-edit", "pre-push"),
    );
    write(
        root.path(),
        "packs/nix/pack.toml",
        "name = 'nix'\ndescription = 'Nix'",
    );
    for (name, tier, path, rule) in [
        ("gate", "gate", "**/*.nix", "diff_regex = ['dangerous']"),
        ("note", "advise", "**/*.nix", "diff_regex = ['dangerous']"),
        (
            "removed",
            "gate",
            "**/*.nix",
            "diff_regex = ['removed_only']",
        ),
        (
            "diagnostic",
            "advise",
            "other.rs",
            "diagnostic = ['rustc::E0382']",
        ),
    ] {
        write(root.path(), &format!("packs/nix/cards/{name}.md"), &format!(
            "+++\nid = \"nix/{name}\"\npack = \"nix\"\ntier = \"{tier}\"\ncooldown = \"session\"\n\
             fixture = \"unused.json\"\nevents = [\"pre-push\"]\n[on]\npath = ['{path}']\n{rule}\n+++\nNote\n"
        ));
    }
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(root.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q"]);
    for file in ["src/a.rs", "src/β b.rs"] {
        write(root.path(), file, "fn f() {}\n");
    }
    write(root.path(), "z/flake.nix", "removed_only\n");
    git(&["add", "src", "z"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.org",
        "-c",
        "core.hooksPath=/dev/null",
        "commit",
        "-qm",
        "fixture",
        "--no-gpg-sign",
    ]);
    let sha = git(&["rev-parse", "HEAD"]);
    for file in ["src/a.rs", "src/β b.rs"] {
        write(root.path(), file, "fn f() { x.unwrap(); }\n");
    }
    write(root.path(), "z/flake.nix", "dangerous\nunwrap();\n");
    let payload = json!({"findings":[
        {"code":"rustc::E0382","file":"other.rs"},
        {"code":"clippy::unwrap_used","file":root.path().join("src/a.rs")},
    ]});
    let run = |session, max_cards| {
        let mut cmd = skillnet(root.path());
        if let Some(session) = session {
            cmd.args(["trigger", "--session", session, "--state-dir", "state"]);
        } else {
            cmd.arg("trigger");
        }
        json_output(
            cmd.args([
                "--event",
                "pre-push",
                "--cwd",
                "src",
                "--base",
                sha.trim(),
                "--max-cards",
                max_cards,
                "--json",
            ])
            .write_stdin(payload.to_string()),
        )
    };
    let output = run(None, "3");
    assert_eq!(output["matched_count"], 5);
    assert_eq!(output["collapsed"], json!({"rust/check":1}));
    assert_eq!(output["deny"]["card"], "nix/gate");
    let mut ids: Vec<_> = output["inject"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["card"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, ["nix/diagnostic", "nix/note", "rust/check"]);
    let capped = run(Some("capped"), "1");
    assert_eq!(capped["inject"].as_array().unwrap().len(), 1);
    assert_eq!(capped["budget_dropped"], 3);
    let repeated = run(Some("capped"), "3");
    assert_eq!(
        repeated["collapsed"],
        json!({"nix/diagnostic":1,"rust/check":1})
    );
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
