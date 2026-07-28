//! Private mode integration tests (whetstone-xdr / ze9 / 336).
//!
//! The contract under test: `wh init --claude --private` on a shared repo leaves
//! `git status` EMPTY (nothing for a teammate to see or accidentally commit)
//! while scan/status keep working, tracked files are never modified, and
//! `wh publish` flips the same artifacts into normal trackable files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn whetstone_bin() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    path.pop();
    path.push("whetstone");
    assert!(
        path.exists(),
        "whetstone binary not built at {}",
        path.display()
    );
    path
}

fn temp_repo(label: &str) -> PathBuf {
    let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "wh-private-{label}-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git")
}

fn git_ok(repo: &Path, args: &[&str]) {
    let out = git(repo, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_status_porcelain(repo: &Path) -> String {
    let out = git(repo, &["status", "--porcelain"]);
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Fresh git repo with a committed fastapi manifest — the "shared team repo"
/// a solo adopter walks into.
fn seeded_repo(label: &str) -> PathBuf {
    let repo = temp_repo(label);
    git_ok(&repo, &["init", "-q"]);
    git_ok(&repo, &["config", "user.email", "test@example.com"]);
    git_ok(&repo, &["config", "user.name", "Test"]);
    git_ok(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(
        repo.join("pyproject.toml"),
        "[project]\nname = \"t\"\nversion = \"0.1.0\"\ndependencies = [\"fastapi>=0.110\"]\n",
    )
    .unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "initial"]);
    repo
}

fn run_wh(repo: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(whetstone_bin())
        .args(args)
        .current_dir(repo)
        .env("WHETSTONE_NO_TUI", "1")
        .output()
        .expect("run whetstone");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

#[test]
fn private_init_zero_footprint_then_publish_flips() {
    let repo = seeded_repo("flip");

    // Private onboarding: the full agent door, hidden.
    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "init --claude --private failed: {out} {err}");
    assert!(out.contains("\"enabled\""), "expected private enable: {out}");

    // THE invariant: a teammate running `git status` sees nothing.
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "private init must leave git status empty"
    );
    // ...but the artifacts genuinely exist and work.
    assert!(repo.join("whetstone/whetstone.yaml").exists());
    assert!(repo.join(".mcp.json").exists());

    // Enforcement is mode-independent: scan runs (and writes .state) without leaking.
    let (scan_out, _, scan_ok) = run_wh(&repo, &["scan", ".", "--json", "--no-fail"]);
    assert!(scan_ok, "scan in private mode failed: {scan_out}");
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "scan must not leak state past the exclude block"
    );

    // The oracle reports the mode.
    let (setup_out, _, setup_ok) = run_wh(&repo, &["status", "--setup", "--json"]);
    assert!(setup_ok);
    assert!(
        setup_out.contains("\"private_mode\": true") || setup_out.contains("\"private_mode\":true"),
        "status --setup must surface private_mode: {setup_out}"
    );

    // Enable is idempotent.
    let (again, _, again_ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(again_ok);
    assert!(again.contains("\"noop\""), "second enable should noop: {again}");

    // The flip.
    let (pub_out, pub_err, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok, "publish failed: {pub_out} {pub_err}");
    assert!(pub_out.contains("\"published\""));
    assert!(
        pub_out.contains("git add"),
        "publish must print the git add list: {pub_out}"
    );

    // Exclude block gone; artifacts now visible to git; machine-local state ignored.
    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap_or_default();
    assert!(
        !exclude.contains("whetstone"),
        "managed block must be removed: {exclude}"
    );
    let status = git_status_porcelain(&repo);
    assert!(
        status.contains("whetstone/"),
        "artifacts must be trackable after publish: {status}"
    );
    assert!(
        !status.contains(".state"),
        ".state must stay ignored via .gitignore after publish: {status}"
    );
    let gitignore = std::fs::read_to_string(repo.join(".gitignore")).unwrap();
    assert!(gitignore.contains("whetstone/.state/"));

    // Publish is idempotent.
    let (pub2, _, pub2_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub2_ok);
    assert!(pub2.contains("\"noop\""), "second publish should noop: {pub2}");

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn tracked_wiring_files_are_never_modified() {
    let repo = seeded_repo("tracked");

    // The team already committed an .mcp.json and .claude/settings.json.
    let mcp_body = "{\n  \"mcpServers\": {\n    \"other\": { \"command\": \"other\" }\n  }\n}\n";
    std::fs::write(repo.join(".mcp.json"), mcp_body).unwrap();
    std::fs::create_dir_all(repo.join(".claude")).unwrap();
    let settings_body = "{\n  \"model\": \"opus\"\n}\n";
    std::fs::write(repo.join(".claude/settings.json"), settings_body).unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "team wiring"]);

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "init failed: {out} {err}");

    // Tracked files: byte-identical. Exclude can't hide tracked-file changes,
    // so private mode must not have touched them.
    assert_eq!(
        std::fs::read_to_string(repo.join(".mcp.json")).unwrap(),
        mcp_body,
        "tracked .mcp.json must not be modified in private mode"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap(),
        settings_body,
        "tracked settings.json must not be modified in private mode"
    );
    assert_eq!(git_status_porcelain(&repo), "");

    // The report surfaces the local-scope MCP alternative; hooks landed in
    // settings.local.json instead.
    assert!(out.contains("claude mcp add"), "must print local-scope alternative: {out}");
    let local = std::fs::read_to_string(repo.join(".claude/settings.local.json")).unwrap();
    assert!(local.contains("whetstone-posttooluse-hook.sh"));

    // Publish completes the shared wiring and migrates our hooks out of local.
    let (pub_out, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok, "publish failed: {pub_out}");
    let mcp_after = std::fs::read_to_string(repo.join(".mcp.json")).unwrap();
    assert!(mcp_after.contains("whetstone"), "publish must register MCP: {mcp_after}");
    assert!(mcp_after.contains("\"other\""), "existing servers must survive");
    let settings_after = std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap();
    assert!(settings_after.contains("whetstone-posttooluse-hook.sh"));
    assert!(settings_after.contains("\"model\""), "user settings must survive");
    let local_after = std::fs::read_to_string(repo.join(".claude/settings.local.json")).unwrap();
    assert!(
        !local_after.contains("whetstone-"),
        "publish must migrate our hooks out of settings.local.json: {local_after}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn ci_is_refused_in_private_mode() {
    let repo = seeded_repo("ci");
    let (_, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(ok);

    let (out, _, ci_ok) = run_wh(&repo, &["init", "--ci", "--json"]);
    assert!(!ci_ok, "init --ci must fail in private mode: {out}");
    assert!(out.contains("private"), "error must explain why: {out}");
    assert!(
        !repo.join(".github/workflows/whetstone-check.yml").exists(),
        "no workflow file may be written in private mode"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn private_refused_when_whetstone_already_tracked() {
    let repo = seeded_repo("already");
    std::fs::create_dir_all(repo.join("whetstone")).unwrap();
    std::fs::write(repo.join("whetstone/whetstone.yaml"), "version: 1\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "already onboarded"]);

    let (out, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(!ok, "must refuse when whetstone/ is tracked: {out}");
    assert!(out.contains("already"), "error must explain: {out}");

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn private_refused_outside_git_repo() {
    let dir = temp_repo("nogit");
    let (out, _, ok) = run_wh(&dir, &["init", "--private", "--json"]);
    assert!(!ok, "must refuse outside a git repo: {out}");
    assert!(out.contains("git"), "error must mention git: {out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn user_content_in_exclude_file_is_preserved() {
    let repo = seeded_repo("exclude");
    let info = repo.join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("exclude"), "# my stuff\nscratch/\n").unwrap();

    let (_, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(ok);
    let during = std::fs::read_to_string(info.join("exclude")).unwrap();
    assert!(during.contains("scratch/"));
    assert!(during.contains("/whetstone/"));

    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);
    let after = std::fs::read_to_string(info.join("exclude")).unwrap();
    assert!(after.contains("scratch/"), "user content must survive publish: {after}");
    assert!(!after.contains("/whetstone/"), "managed entries must be gone: {after}");

    std::fs::remove_dir_all(&repo).ok();
}
