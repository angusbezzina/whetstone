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
    // MINOR 7: no hollow settings.local.json left behind as a new untracked file.
    let local_path = repo.join(".claude/settings.local.json");
    if local_path.exists() {
        let local_after = std::fs::read_to_string(&local_path).unwrap();
        assert!(
            !local_after.contains("whetstone-"),
            "publish must migrate our hooks out of settings.local.json: {local_after}"
        );
    }
    let after_status = git_status_porcelain(&repo);
    assert!(
        !after_status.contains("settings.local.json"),
        "publish must not introduce a hollow settings.local.json: {after_status}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// MAJOR 1 regression: the exclude file lives at the REPO ROOT, so a package
/// inside a monorepo needs its path prefix on every entry. Root-anchored
/// entries matched nothing and exposed every artifact while reporting success.
#[test]
fn monorepo_subdirectory_has_zero_footprint() {
    let repo = seeded_repo("monorepo");
    let pkg = repo.join("packages/api");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(
        pkg.join("pyproject.toml"),
        "[project]\nname = \"api\"\nversion = \"0.1.0\"\ndependencies = [\"fastapi>=0.110\"]\n",
    )
    .unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "package"]);

    let (out, err, ok) = run_wh(&pkg, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "private init in a subdirectory failed: {out} {err}");
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "a monorepo package must have the same zero footprint as a root repo"
    );
    assert!(pkg.join("whetstone/whetstone.yaml").exists(), "artifacts still written");

    // The block must be anchored under the package, not the repo root.
    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    assert!(
        exclude.contains("/packages/api/whetstone/"),
        "entries must carry the package prefix: {exclude}"
    );

    // And publish must flip it back correctly from the same directory.
    let (pub_out, _, pub_ok) = run_wh(&pkg, &["publish", "--json"]);
    assert!(pub_ok, "publish from a subdirectory failed: {pub_out}");
    let status = git_status_porcelain(&repo);
    assert!(status.contains("packages/api/whetstone/"), "artifacts trackable: {status}");
    let exclude_after = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    assert!(!exclude_after.contains("whetstone"), "block removed: {exclude_after}");

    std::fs::remove_dir_all(&repo).ok();
}

/// MAJOR A regression (round 2): two packages in one monorepo, both private.
/// The block was marker-fenced but not labelled, so the second enable replaced
/// the first's block — silently re-exposing a package Whetstone still reported
/// as private. Publish had the mirror bug.
#[test]
fn two_private_packages_coexist_in_one_repo() {
    let repo = seeded_repo("twopkg");
    for name in ["api", "web"] {
        let pkg = repo.join("packages").join(name);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            "{\n  \"name\": \"x\",\n  \"dependencies\": { \"react\": \"^18.0.0\" }\n}\n",
        )
        .unwrap();
    }
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "packages"]);

    let api = repo.join("packages/api");
    let web = repo.join("packages/web");

    let (out_a, err_a, ok_a) = run_wh(&api, &["init", "--claude", "--private", "--json"]);
    assert!(ok_a, "api private init failed: {out_a} {err_a}");
    assert_eq!(git_status_porcelain(&repo), "", "api must be hidden");

    let (out_w, err_w, ok_w) = run_wh(&web, &["init", "--claude", "--private", "--json"]);
    assert!(ok_w, "web private init failed: {out_w} {err_w}");
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "enabling web must not un-hide api"
    );

    // Publishing one package must leave the other private.
    let (pub_a, _, pub_a_ok) = run_wh(&api, &["publish", "--json"]);
    assert!(pub_a_ok, "api publish failed: {pub_a}");
    let status = git_status_porcelain(&repo);
    assert!(
        status.contains("packages/api/whetstone/"),
        "api must become trackable: {status}"
    );
    assert!(
        !status.contains("packages/web/whetstone/"),
        "web must stay hidden after api publishes: {status}"
    );

    // ...and web can still publish itself afterwards.
    let (pub_w, _, pub_w_ok) = run_wh(&web, &["publish", "--json"]);
    assert!(pub_w_ok, "web publish failed: {pub_w}");
    let status2 = git_status_porcelain(&repo);
    assert!(status2.contains("packages/web/whetstone/"), "web trackable: {status2}");
    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap_or_default();
    assert!(!exclude.contains("whetstone"), "no blocks left: {exclude}");

    std::fs::remove_dir_all(&repo).ok();
}

/// F1 regression (round 3): the label is delimited by `[...]`, so a path
/// containing `]` truncated on read — publish could not find its own block
/// (artifacts stayed hidden while it reported success) and a sibling whose
/// label was a prefix of the truncation had its block deleted.
#[test]
fn bracketed_package_paths_round_trip() {
    let repo = seeded_repo("brackets");
    for name in ["a", "a]b", "pkg[1]"] {
        let pkg = repo.join(name);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            "{\n  \"name\": \"x\",\n  \"dependencies\": { \"react\": \"^18.0.0\" }\n}\n",
        )
        .unwrap();
    }
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "packages"]);

    for name in ["a]b", "pkg[1]", "a"] {
        let (out, err, ok) = run_wh(&repo.join(name), &["init", "--claude", "--private", "--json"]);
        assert!(ok, "private init failed for {name}: {out} {err}");
        assert_eq!(
            git_status_porcelain(&repo),
            "",
            "enabling {name} must hide it and leave siblings hidden"
        );
    }

    // Re-running must be a noop, not an appended duplicate block.
    let (again, _, again_ok) = run_wh(&repo.join("pkg[1]"), &["init", "--private", "--json"]);
    assert!(again_ok);
    assert!(again.contains("\"noop\""), "re-enable must find its own block: {again}");

    // Publish must remove its own block and actually expose that package.
    let (pub_out, _, pub_ok) = run_wh(&repo.join("pkg[1]"), &["publish", "--json"]);
    assert!(pub_ok, "publish failed: {pub_out}");
    let status = git_status_porcelain(&repo);
    assert!(
        status.contains("pkg[1]/whetstone/"),
        "publish must make the package trackable: {status}"
    );
    assert!(
        !status.contains("a]b/whetstone/") && !status.contains("\na/whetstone/"),
        "siblings must stay hidden: {status}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// F2 regression (round 3): enable is a read-modify-write of one shared file.
/// Concurrent onboarding of two packages lost a block every time, leaving a
/// package with `setup.private: true` and nothing actually hidden.
#[test]
fn concurrent_enable_keeps_every_block() {
    let repo = seeded_repo("concurrent");
    let names = ["p1", "p2", "p3", "p4"];
    for name in names {
        let pkg = repo.join(name);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            "{\n  \"name\": \"x\",\n  \"dependencies\": { \"react\": \"^18.0.0\" }\n}\n",
        )
        .unwrap();
    }
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "packages"]);

    let handles: Vec<_> = names
        .iter()
        .map(|name| {
            let dir = repo.join(name);
            std::thread::spawn(move || run_wh(&dir, &["init", "--claude", "--private", "--json"]))
        })
        .collect();
    for (name, h) in names.iter().zip(handles) {
        let (out, err, ok) = h.join().expect("thread");
        assert!(ok, "concurrent init failed for {name}: {out} {err}");
    }

    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "every concurrently-enabled package must still be hidden"
    );
    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    for name in names {
        assert!(
            exclude.contains(&format!("/{name}/whetstone/")),
            "{name}'s block was lost: {exclude}"
        );
    }

    std::fs::remove_dir_all(&repo).ok();
}

/// F3 regression (round 3): a torn block (no terminator) made strip_block eat
/// every user line after it, violating the stated preserve-verbatim invariant.
#[test]
fn torn_block_preserves_user_lines_below_it() {
    let repo = seeded_repo("tornuser");
    let info = repo.join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(
        info.join("exclude"),
        "personal-a/\n# >>> whetstone private mode [.] (managed by `wh`; `wh publish` removes this block) >>>\n/whetstone/\npersonal-b/\npersonal-c/\n",
    )
    .unwrap();

    let (out, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(ok, "init failed: {out}");
    let after = std::fs::read_to_string(info.join("exclude")).unwrap();
    for line in ["personal-a/", "personal-b/", "personal-c/"] {
        assert!(after.contains(line), "user line {line} must survive a repair: {after}");
    }

    std::fs::remove_dir_all(&repo).ok();
}

/// F4 regression (round 3): an in-tree .gitignore negation outranks
/// .git/info/exclude, so the promise silently failed. Private mode now asks git
/// whether it actually holds and fails loudly when it doesn't.
#[test]
fn gitignore_negation_defeating_the_block_fails_loudly() {
    let repo = seeded_repo("negation");
    std::fs::write(repo.join(".gitignore"), ".claude/*\n!.claude/settings.json\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "team gitignore"]);

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        !ok,
        "a defeated exclude must be a hard failure, not silent success: {out} {err}"
    );
    assert!(
        out.contains("exposed_artifacts") || err.contains("NOT in effect"),
        "the exposed path must be named: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// A regression (round 4): git C-quotes any path with non-ASCII bytes, a quote
/// or a backslash (core.quotePath defaults to true). The verifier parsed
/// unquoted paths, so a leak under such a path was filtered away and reported
/// as `"verified": true`. `-z` output is never quoted.
#[test]
fn verification_sees_leaks_under_quoted_paths() {
    for name in ["café", "qu\"ote", "back\\slash"] {
        let repo = seeded_repo("quoted");
        let pkg = repo.join(name);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            "{\n  \"name\": \"x\",\n  \"dependencies\": { \"react\": \"^18.0.0\" }\n}\n",
        )
        .unwrap();
        // Defeat the exclude from inside the package.
        std::fs::write(pkg.join(".gitignore"), "!whetstone/\n").unwrap();
        git_ok(&repo, &["add", "-A"]);
        git_ok(&repo, &["commit", "-q", "-m", "pkg"]);

        let (out, err, ok) = run_wh(&pkg, &["init", "--claude", "--private", "--json"]);
        assert!(
            !ok,
            "a leak under a git-quoted path ({name}) must fail loudly: {out} {err}"
        );
        std::fs::remove_dir_all(&repo).ok();
    }
}

/// B regression (round 4): a control character in a path component splits the
/// fence line and every entry, corrupting the block — and it leaked with no
/// user misconfiguration at all. Refuse instead of writing something broken.
#[test]
fn control_characters_in_the_path_are_refused() {
    let repo = seeded_repo("ctrl");
    let pkg = repo.join("a\nb");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        "{\n  \"name\": \"x\",\n  \"dependencies\": { \"react\": \"^18.0.0\" }\n}\n",
    )
    .unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "pkg"]);

    let (out, err, ok) = run_wh(&pkg, &["init", "--claude", "--private", "--json"]);
    assert!(!ok, "a control char in the path must be refused: {out} {err}");
    assert!(
        out.contains("control character") || err.contains("control character"),
        "the refusal must explain why: {out} {err}"
    );
    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap_or_default();
    assert!(!exclude.contains("whetstone"), "nothing may be written: {exclude}");

    std::fs::remove_dir_all(&repo).ok();
}

/// C regression (round 4): the verifier returned "nothing exposed" when git
/// status could not run — a check that cannot run must not read as a pass.
#[test]
fn unverifiable_private_mode_is_an_error() {
    let repo = seeded_repo("unverifiable");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "baseline private init should succeed");

    // Break git's index so `git status` cannot run.
    let index = repo.join(".git/index");
    std::fs::write(&index, "not an index").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o000)).unwrap();
    }

    let (out, err, ok2) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(
        !ok2,
        "an unverifiable private mode must be an error, not a silent pass: {out} {err}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::remove_dir_all(&repo).ok();
}

/// D regression (round 4): a foreign line inside the block made publish leave
/// the rest of the block behind — so the printed `git add` command failed with
/// "paths are ignored by one of your .gitignore files".
#[test]
fn publish_is_a_clean_inverse_even_with_a_foreign_line_in_the_block() {
    let repo = seeded_repo("foreign");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);

    // User parks a line inside the managed block.
    let exclude_path = repo.join(".git/info/exclude");
    let content = std::fs::read_to_string(&exclude_path).unwrap();
    let patched = content.replace("/.mcp.json\n", "/.mcp.json\nmy-own-ignore/\n");
    std::fs::write(&exclude_path, patched).unwrap();

    let (_, _, ok2) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(ok2, "re-enable should repair");
    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);

    let after = std::fs::read_to_string(&exclude_path).unwrap();
    assert!(
        !after.contains("whetstone") && !after.contains("/.claude/"),
        "publish must leave no managed residue: {after}"
    );
    assert!(after.contains("my-own-ignore/"), "user line must survive: {after}");

    // The artifacts publish points at must actually be addable.
    let status = git_status_porcelain(&repo);
    assert!(status.contains("whetstone/"), "artifacts trackable: {status}");
    let add = git(&repo, &["add", ".claude/settings.json"]);
    assert!(
        add.status.success(),
        "publish's own git add must work: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-5 #1 regression: git treats `.git/info/exclude` as BYTES. A single
/// non-UTF-8 byte made `read_to_string` fail and the file be treated as empty —
/// destroying the user's personal ignores on enable, and turning publish into a
/// silent no-op that reported success and printed a failing `git add`.
#[test]
fn non_utf8_exclude_file_is_never_silently_discarded() {
    let repo = seeded_repo("nonutf8");
    let info = repo.join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    let original: Vec<u8> = b"# my personal ignores\n/secret-notes/\n/caf\xe9-cache/\n*.bak\n".to_vec();
    std::fs::write(info.join("exclude"), &original).unwrap();

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        !ok,
        "a non-UTF-8 exclude file must be an error, not a silent rewrite: {out} {err}"
    );
    assert_eq!(
        std::fs::read(info.join("exclude")).unwrap(),
        original,
        "the user's personal ignores must survive byte-for-byte"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-5 #2 regression: the CI workflow is a Whetstone artifact that is
/// inherently shared, so it is never hidden — but it was also missing from the
/// verifier, so `wh init --ci` then `--private` reported "invisible to git
/// status" with a Whetstone-written file plainly visible.
#[test]
fn a_visible_ci_workflow_is_not_reported_as_invisible() {
    let repo = seeded_repo("ciworkflow");
    let (_, _, ci_ok) = run_wh(&repo, &["init", "--ci", "--json"]);
    assert!(ci_ok, "writing the workflow before going private is allowed");
    assert!(repo.join(".github/workflows/whetstone-check.yml").exists());

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        !ok,
        "a visible Whetstone artifact must not be reported as verified: {out} {err}"
    );
    assert!(
        out.contains("whetstone-check.yml") || err.contains("whetstone-check.yml"),
        "the exposed workflow must be named: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-5 #3 regression: worktrees of one clone SHARE `.git/info/exclude`, so
/// publishing in one un-hides the others. That has to be said out loud.
#[test]
fn shared_worktree_exclude_is_warned_about() {
    let repo = seeded_repo("wtwarn");
    let wt = repo.parent().unwrap().join(format!(
        "{}-wt2",
        repo.file_name().unwrap().to_string_lossy()
    ));
    git_ok(
        &repo,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "feat"],
    );

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "private init with a worktree present should succeed: {out} {err}");
    assert!(
        out.contains("worktree"),
        "the shared-exclude caveat must be surfaced: {out}"
    );

    let (pub_out, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);
    assert!(
        pub_out.contains("worktree"),
        "publish must warn it affects every worktree: {pub_out}"
    );

    git(&repo, &["worktree", "remove", "--force", wt.to_str().unwrap()]);
    std::fs::remove_dir_all(&repo).ok();
    std::fs::remove_dir_all(&wt).ok();
}

/// Round-5 #5 regression: writing through a symlinked exclude that points at a
/// TRACKED worktree file would modify a tracked file — invisibly to the
/// artifact-scoped verifier.
#[test]
fn exclude_symlinked_to_a_tracked_file_is_refused() {
    let repo = seeded_repo("symtracked");
    std::fs::write(repo.join("ignores.txt"), "# team ignores\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "tracked ignores"]);

    let info = repo.join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    let link = info.join("exclude");
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../ignores.txt", &link).unwrap();

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(!ok, "must refuse to write through a symlink to a tracked file: {out} {err}");
    assert_eq!(
        std::fs::read_to_string(repo.join("ignores.txt")).unwrap(),
        "# team ignores\n",
        "the tracked file must be untouched"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-6 MAJOR-1 regression: `wh init --personal` writes a Whetstone block
/// into `.gitignore`, which no ignore mechanism can hide. Enabling private mode
/// afterwards reported "invisible to git status" with that file visible.
#[test]
fn whetstone_written_gitignore_is_not_reported_invisible() {
    let repo = seeded_repo("gitignore");
    let (_, _, personal_ok) = run_wh(&repo, &["init", "--personal", "--json"]);
    assert!(personal_ok, "public --personal writes .gitignore");
    assert!(
        git_status_porcelain(&repo).contains(".gitignore"),
        "precondition: .gitignore is visible"
    );

    // Round-10 correction: this is ADVISORY, not fatal. The file holds ignore
    // lines only (no rules or config) and is legitimately shared — blocking on it
    // made `enable → publish → enable` impossible. It must still be NAMED, so
    // `wh` never claims a visible artifact is invisible.
    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        ok,
        "a Whetstone-written .gitignore is advisory, not blocking: {out} {err}"
    );
    assert!(
        out.contains(".gitignore") || err.contains(".gitignore"),
        "the exposed file must still be named: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-6 MODERATE-1 regression: a tracked artifact with the user's OWN
/// uncommitted edit is not our leak (`skip_tracked` means we never wrote it),
/// but it hard-failed enable and wedged the repo — in exactly the
/// "trial Whetstone on the team's repo" case private mode exists for.
#[test]
fn a_users_own_edit_to_a_tracked_artifact_is_not_a_leak() {
    let repo = seeded_repo("useredit");
    std::fs::create_dir_all(repo.join(".claude")).unwrap();
    std::fs::write(repo.join(".claude/settings.json"), "{\n  \"model\": \"opus\"\n}\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "team settings"]);
    // The user tweaks the tracked file themselves, before adopting Whetstone.
    std::fs::write(
        repo.join(".claude/settings.json"),
        "{\n  \"model\": \"opus\",\n  \"mine\": true\n}\n",
    )
    .unwrap();

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        ok,
        "the user's own edit to a tracked file must not fail private mode: {out} {err}"
    );
    // Their edit is untouched and still theirs.
    assert!(std::fs::read_to_string(repo.join(".claude/settings.json"))
        .unwrap()
        .contains("\"mine\""));
    let status = git_status_porcelain(&repo);
    assert!(
        status.contains(".claude/settings.json") && !status.contains("whetstone/"),
        "only the user's own edit shows; no Whetstone artifact leaks: {status}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-6 MINOR-1 regression: publish round-tripped the exclude file through
/// `lines()`, converting CRLF to LF and forcing a trailing newline.
#[test]
fn publish_preserves_line_endings_byte_for_byte() {
    let repo = seeded_repo("crlf");
    let info = repo.join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    let original = "# personal\r\n*.log\r\nnotrailing";
    std::fs::write(info.join("exclude"), original).unwrap();

    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);
    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);

    // CRLF endings survive; the one documented normalization is that a file
    // with no final newline gains one (enable must add a separator before its
    // block, and publish cannot know afterwards whether it was already there).
    assert_eq!(
        std::fs::read_to_string(info.join("exclude")).unwrap(),
        format!("{original}\n"),
        "publish must restore the exclude file, preserving CRLF"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-6 MINOR-4 regression: publish calls install_hooks implicitly, which
/// wrote `.githooks/post-merge` wholesale — destroying a team's committed hook.
#[test]
fn a_tracked_team_post_merge_hook_is_never_overwritten() {
    let repo = seeded_repo("teamhook");
    std::fs::create_dir_all(repo.join(".githooks")).unwrap();
    let body = "#!/bin/sh\necho team-post-merge\n";
    std::fs::write(repo.join(".githooks/post-merge"), body).unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "team hook"]);

    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);
    let (pub_out, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok, "publish should succeed: {pub_out}");

    assert_eq!(
        std::fs::read_to_string(repo.join(".githooks/post-merge")).unwrap(),
        body,
        "the team's committed hook must survive publish"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-7 MAJOR regression: a STAGED artifact (`A `) is neither untracked nor
/// in HEAD. Using index membership as the ours/theirs test dropped it from the
/// leak set entirely — reachable by following `wh publish`'s own printed
/// `git add` and then re-running `wh init --private`.
#[test]
fn staged_artifacts_are_still_reported_as_exposed() {
    let repo = seeded_repo("staged");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);

    // The user stages artifacts (exactly what publish's next_command says).
    git_ok(&repo, &["add", "-f", ".mcp.json", ".claude"]);
    assert!(
        git_status_porcelain(&repo).contains("A "),
        "precondition: artifacts are staged"
    );

    let (out, err, ok2) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        !ok2,
        "staged artifacts are visible to git and must be reported: {out} {err}"
    );
    assert!(
        out.contains(".mcp.json") || err.contains(".mcp.json"),
        "the staged path must be named: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-7 MODERATE regression: once Whetstone's `.gitignore` block is
/// COMMITTED, it is public history — a later edit by the user is theirs, not
/// ours, and must not wedge every `wh init`.
#[test]
fn a_committed_gitignore_block_plus_user_edit_is_not_a_leak() {
    let repo = seeded_repo("committedgi");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);
    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);

    // The team commits the whetstone gitignore entries, then starts fresh.
    git_ok(&repo, &["add", ".gitignore"]);
    git_ok(&repo, &["commit", "-q", "-m", "adopt whetstone gitignore"]);
    for p in ["whetstone", ".mcp.json", ".claude", ".cursor", ".githooks"] {
        let path = repo.join(p);
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
    }

    let (out1, err1, ok1) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok1, "a clean committed block must pass: {out1} {err1}");

    // The user makes their own unrelated edit to the tracked .gitignore.
    let mut gi = std::fs::read_to_string(repo.join(".gitignore")).unwrap();
    gi.push_str(".env\n");
    std::fs::write(repo.join(".gitignore"), gi).unwrap();

    let (out2, err2, ok2) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        ok2,
        "the user's own edit to an already-committed block must not fail: {out2} {err2}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-7 minor: authorship of `.githooks/post-merge` is detected by marker,
/// not byte equality — otherwise any future edit to the template permanently
/// freezes the hook for every team that committed it.
#[test]
fn an_older_whetstone_post_merge_hook_is_still_updatable() {
    let repo = seeded_repo("oldhook");
    let (_, _, ok) = run_wh(&repo, &["init", "--hooks", "--json"]);
    assert!(ok);
    // Simulate an older release's body: ours plus an extra line.
    let mut body = std::fs::read_to_string(repo.join(".githooks/post-merge")).unwrap();
    body.push_str("# an older release wrote this line\n");
    std::fs::write(repo.join(".githooks/post-merge"), &body).unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "commit whetstone hook"]);

    let (out, _, ok2) = run_wh(&repo, &["init", "--hooks", "--json"]);
    assert!(ok2, "re-running hooks should succeed: {out}");
    assert!(
        !out.contains("was not written by Whetstone"),
        "our own hook must not be misattributed: {out}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-8 MAJOR regression: intent-to-add (`git add -N`) reports ` A` — the
/// `A` sits in the WORKTREE column, so a predicate reading only the index
/// column missed it. The attribution test now asks HEAD directly.
#[test]
fn intent_to_add_artifacts_are_reported_as_exposed() {
    let repo = seeded_repo("intent");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--json"]);
    assert!(ok, "public onboard first");
    git_ok(&repo, &["add", "-N", ".mcp.json"]);

    let (out, err, ok2) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        !ok2,
        "an intent-to-add artifact is visible and must be reported: {out} {err}"
    );
    assert!(
        out.contains(".mcp.json") || err.contains(".mcp.json"),
        "the path must be named: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-8 MODERATE regression: an unmerged `AA` on an artifact that IS in HEAD
/// was misread as ours (both status letters are `A`), hard-failing every
/// `wh init` for the duration of the user's merge conflict.
#[test]
fn an_unmerged_tracked_artifact_is_not_our_leak() {
    let repo = seeded_repo("unmerged");
    // Both branches independently add .mcp.json — a plausible MCP config clash.
    std::fs::write(repo.join(".mcp.json"), "{\n  \"mcpServers\": { \"a\": {} }\n}\n").unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "main mcp"]);
    git_ok(&repo, &["checkout", "-q", "-b", "feat", "HEAD~1"]);
    std::fs::write(repo.join(".mcp.json"), "{\n  \"mcpServers\": { \"b\": {} }\n}\n").unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "feat mcp"]);
    git_ok(&repo, &["checkout", "-q", "main"]);

    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "baseline private init");

    // Create the conflict: .mcp.json is now unmerged (AA), but it is in HEAD.
    let merge = git(&repo, &["merge", "feat"]);
    assert!(!merge.status.success(), "expected a conflict");
    assert!(git_status_porcelain(&repo).contains("AA") || git_status_porcelain(&repo).contains("UU"));

    let (out, err, ok2) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(
        ok2,
        "a tracked artifact in a merge conflict is the user's, not our leak: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-8 minor: porcelain `-z` emits a rename as `R  <new>\0<old>\0`. The
/// bare original-path field was rescanned as a status record, so its first
/// three characters parsed as a status code.
#[test]
fn rename_records_do_not_confuse_the_parser() {
    let repo = seeded_repo("rename");
    // A tracked file whose name ends in an artifact path once 3 chars are cut.
    std::fs::write(repo.join("Abc.mcp.json"), "{}\n").unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "decoy"]);

    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "baseline");
    git_ok(&repo, &["mv", "Abc.mcp.json", "renamed.json"]);

    let (out, err, ok2) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(
        ok2,
        "a rename of an unrelated file must not read as an exposed artifact: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-8 minor: an UNTRACKED user hook has no committed copy to recover
/// from, so overwriting it is worse than the tracked case, not better.
#[test]
fn an_untracked_user_post_merge_hook_is_never_overwritten() {
    let repo = seeded_repo("untrackedhook");
    std::fs::create_dir_all(repo.join(".githooks")).unwrap();
    let body = "#!/bin/sh\necho my own hook\n";
    std::fs::write(repo.join(".githooks/post-merge"), body).unwrap();

    let (out, _, _) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert_eq!(
        std::fs::read_to_string(repo.join(".githooks/post-merge")).unwrap(),
        body,
        "the user's own untracked hook must survive: {out}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-9 F1 regression: `git status` paths are repo-root-relative, but the
/// old HEAD probe passed them to a cwd-relative pathspec — so in a monorepo
/// package every check asked about `<prefix><prefix><artifact>`. That both
/// hard-failed the developer's own tweak and could hide a real leak.
#[test]
fn attribution_is_correct_from_a_monorepo_package() {
    let repo = seeded_repo("monoattr");
    let pkg = repo.join("packages/api");
    std::fs::create_dir_all(pkg.join(".claude")).unwrap();
    std::fs::write(
        pkg.join("package.json"),
        "{\n  \"name\": \"api\",\n  \"dependencies\": { \"react\": \"^18.0.0\" }\n}\n",
    )
    .unwrap();
    // The team commits their own settings.json (no Whetstone content).
    std::fs::write(pkg.join(".claude/settings.json"), "{\n  \"model\": \"opus\"\n}\n").unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "team package"]);
    // The developer's own uncommitted tweak.
    std::fs::write(
        pkg.join(".claude/settings.json"),
        "{\n  \"model\": \"opus\",\n  \"mine\": true\n}\n",
    )
    .unwrap();

    let (out, err, ok) = run_wh(&pkg, &["init", "--claude", "--private", "--json"]);
    assert!(
        ok,
        "a package developer's own edit must not fail private mode: {out} {err}"
    );

    // And a genuine leak in the same package is still caught.
    git_ok(&repo, &["add", "-f", "packages/api/.mcp.json"]);
    let (out2, err2, ok2) = run_wh(&pkg, &["init", "--claude", "--private", "--json"]);
    assert!(
        !ok2,
        "a staged Whetstone artifact in a package must be reported: {out2} {err2}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-9 F2 regression: `.claude/settings.local.json` was the one artifact
/// written with no tracked-file guard — so when a team tracked BOTH settings
/// files, private mode modified a tracked file and reported success.
#[test]
fn a_tracked_settings_overlay_is_never_modified() {
    let repo = seeded_repo("overlay");
    std::fs::create_dir_all(repo.join(".claude")).unwrap();
    let settings = "{\n  \"model\": \"opus\"\n}\n";
    let overlay = "{\n  \"env\": { \"MINE\": \"1\" }\n}\n";
    std::fs::write(repo.join(".claude/settings.json"), settings).unwrap();
    std::fs::write(repo.join(".claude/settings.local.json"), overlay).unwrap();
    git_ok(&repo, &["add", "-Af"]);
    git_ok(&repo, &["commit", "-q", "-m", "team tracks both"]);

    let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "init should succeed, reporting what it skipped: {out} {err}");
    assert_eq!(
        std::fs::read_to_string(repo.join(".claude/settings.local.json")).unwrap(),
        overlay,
        "a tracked overlay must not be modified"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap(),
        settings,
        "the tracked settings file must not be modified"
    );
    assert_eq!(git_status_porcelain(&repo), "", "no visible change at all");
    assert!(
        out.contains("both git-tracked") || out.contains("NOT installed"),
        "the skip must be reported honestly: {out}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// Round-9 F3 regression: the user's OWN staged file that Whetstone never
/// touched was called our leak, with advice (`git restore --staged`) that would
/// have discarded their work. Content decides authorship now.
#[test]
fn the_users_own_staged_file_is_not_our_leak() {
    let repo = seeded_repo("userstaged");
    // The developer writes and stages their own .mcp.json, unrelated to us.
    std::fs::write(
        repo.join(".mcp.json"),
        "{\n  \"mcpServers\": { \"theirs\": { \"command\": \"x\" } }\n}\n",
    )
    .unwrap();
    git_ok(&repo, &["add", "-f", ".mcp.json"]);

    let (out, err, ok) = run_wh(&repo, &["init", "--hooks", "--private", "--json"]);
    assert!(
        ok,
        "the user's own staged file must not be reported as our leak: {out} {err}"
    );
    assert!(
        std::fs::read_to_string(repo.join(".mcp.json")).unwrap().contains("theirs"),
        "and it must be untouched"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// MAJOR B regression (round 2): in a linked worktree `.git` is a FILE, so the
/// naive `.git/hooks` probe reported "no hooks" and core.hooksPath was written
/// into the SHARED config — disabling the main worktree's live pre-commit.
#[test]
fn worktree_does_not_disable_shared_git_hooks() {
    let repo = seeded_repo("wt");
    let pre_commit = repo.join(".git/hooks/pre-commit");
    std::fs::write(&pre_commit, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&pre_commit).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&pre_commit, p).unwrap();
    }

    let wt = repo.parent().unwrap().join(format!(
        "{}-linked",
        repo.file_name().unwrap().to_string_lossy()
    ));
    git_ok(
        &repo,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "feature"],
    );

    let (out, err, ok) = run_wh(&wt, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "init in a worktree failed: {out} {err}");

    let cfg = git(&repo, &["config", "--get", "core.hooksPath"]);
    let value = String::from_utf8_lossy(&cfg.stdout).trim().to_string();
    assert!(
        value.is_empty(),
        "a worktree must not redirect the shared core.hooksPath (got {value})"
    );
    assert!(
        out.contains("core.hooksPath"),
        "the skip must be reported, not silent: {out}"
    );

    // The main worktree's gate still fires.
    std::fs::write(repo.join("y.txt"), "y").unwrap();
    git_ok(&repo, &["add", "y.txt"]);
    let commit = git(&repo, &["commit", "-m", "blocked"]);
    assert!(!commit.status.success(), "pre-commit must still block");

    git(&repo, &["worktree", "remove", "--force", wt.to_str().unwrap()]);
    std::fs::remove_dir_all(&repo).ok();
    std::fs::remove_dir_all(&wt).ok();
}

/// MINOR F regression: publish must not ship `setup.private: false` to the
/// whole team in the file it makes trackable.
#[test]
fn publish_leaves_no_private_marker_behind() {
    let repo = seeded_repo("marker");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);
    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);

    let yaml = std::fs::read_to_string(repo.join("whetstone/whetstone.yaml")).unwrap();
    assert!(
        !yaml.contains("private"),
        "published config must not carry the private marker: {yaml}"
    );
    // And no config-key warnings on a normal command.
    let (out, err, _) = run_wh(&repo, &["status", "--setup", "--json"]);
    assert!(
        !out.contains("unknown config key") && !err.contains("unknown config key"),
        "setup keys must be known to the config validator: {out} {err}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// MAJOR 2 regression: every artifact private mode can write must be skipped
/// when tracked — not just .mcp.json and settings.json. Overwriting a
/// teammate's committed .githooks/post-merge destroyed their content.
#[test]
fn every_tracked_artifact_is_left_untouched() {
    let cases: &[(&str, &str)] = &[
        (".githooks/post-merge", "#!/bin/sh\necho team-post-merge\n"),
        (".cursor/whetstone-session.md", "# team cursor notes\n"),
        (".claude/whetstone-session-hook.sh", "#!/bin/sh\necho team-session\n"),
        (
            ".claude/whetstone-posttooluse-hook.sh",
            "#!/bin/sh\necho team-posttool\n",
        ),
    ];

    for (rel, body) in cases {
        let repo = seeded_repo("guard");
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        git_ok(&repo, &["add", "."]);
        git_ok(&repo, &["commit", "-q", "-m", "team file"]);

        let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
        assert!(ok, "init failed with tracked {rel}: {out} {err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            *body,
            "tracked {rel} must not be modified in private mode"
        );
        assert_eq!(
            git_status_porcelain(&repo),
            "",
            "tracked {rel} produced a visible diff"
        );
        std::fs::remove_dir_all(&repo).ok();
    }
}

/// MAJOR 3 regression: setting core.hooksPath silently disables every hook in
/// .git/hooks/ (the `pre-commit install` layout), with no signal to the user.
#[test]
fn existing_git_hooks_are_not_disabled() {
    let repo = seeded_repo("hookspath");
    let pre_commit = repo.join(".git/hooks/pre-commit");
    std::fs::write(&pre_commit, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&pre_commit).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&pre_commit, p).unwrap();
    }

    let (out, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "init failed: {out}");

    let cfg = git(&repo, &["config", "--get", "core.hooksPath"]);
    let value = String::from_utf8_lossy(&cfg.stdout).trim().to_string();
    assert!(
        value.is_empty(),
        "core.hooksPath must not be redirected away from live .git/hooks (got {value})"
    );
    assert!(
        out.contains("core.hooksPath"),
        "the situation must be reported, not silent: {out}"
    );
    // Proof the user's gate still fires.
    std::fs::write(repo.join("x.txt"), "x").unwrap();
    git_ok(&repo, &["add", "x.txt"]);
    let commit = git(&repo, &["commit", "-m", "should be blocked"]);
    assert!(
        !commit.status.success(),
        "the pre-commit hook must still block the commit"
    );

    std::fs::remove_dir_all(&repo).ok();
}

/// MINOR 4 regression: a torn/stale block was trusted on the BEGIN marker
/// alone, so a re-run left artifacts exposed instead of repairing it.
#[test]
fn enable_repairs_a_torn_block() {
    let repo = seeded_repo("torn");
    let info = repo.join(".git/info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(
        info.join("exclude"),
        "user-stuff\n# >>> whetstone private mode (managed by `wh`; `wh publish` removes this block) >>>\n/whetstone/\n",
    )
    .unwrap();

    let (out, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok, "init failed: {out}");
    assert!(out.contains("\"repaired\""), "torn block must be repaired: {out}");
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "a repaired block must hide every artifact"
    );
    let exclude = std::fs::read_to_string(info.join("exclude")).unwrap();
    assert!(exclude.contains("user-stuff"), "user content preserved: {exclude}");

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

// ── round-10 regressions ──

/// The user's own `whetstone/` ignore line is NOT our leak. This is the most
/// natural first move a cautious solo adopter makes before running the tool, and
/// it used to hard-refuse onboarding with a false diagnosis — leaving the repo
/// flagged private with nothing onboarded.
#[test]
fn users_own_gitignore_whetstone_line_does_not_block_enable() {
    for line in ["whetstone/\n", "whetstone-scratch/\n", "# TODO: try whetstone\n"] {
        let repo = seeded_repo("giown");
        let mut gi = std::fs::read_to_string(repo.join(".gitignore")).unwrap_or_default();
        gi.push_str(line);
        std::fs::write(repo.join(".gitignore"), &gi).unwrap();

        let (out, err, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
        assert!(
            ok,
            "a user-authored {line:?} must not block private mode: {out} {err}"
        );
        // Fully onboarded, not half-written.
        assert!(repo.join("whetstone/packs").exists(), "packs must exist: {out}");
        // The user's own .gitignore edit is the ONLY thing git can see.
        let status = git_status_porcelain(&repo);
        assert!(
            status.ends_with(".gitignore") && status.lines().count() == 1,
            "only the user's own .gitignore may be visible, got: {status:?}"
        );
        std::fs::remove_dir_all(&repo).ok();
    }
}

/// A blocking refusal must not leave the project flagged private with nothing
/// onboarded — enable reverts exactly what it created.
#[test]
fn blocking_refusal_reverts_marker_and_block() {
    let repo = seeded_repo("revert");
    // An inherently-shared CI workflow cannot be hidden → blocking.
    let (_, _, ci_ok) = run_wh(&repo, &["init", "--ci", "--json"]);
    assert!(ci_ok);

    let (out, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(!ok, "must refuse while a CI workflow is present: {out}");

    let ws = std::fs::read_to_string(repo.join("whetstone/whetstone.yaml")).unwrap_or_default();
    assert!(
        !ws.contains("private: true"),
        "marker must be reverted after a blocking refusal: {ws}"
    );
    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap_or_default();
    assert!(
        !exclude.contains("whetstone private mode"),
        "block must be reverted after a blocking refusal: {exclude}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// `enable → publish → enable` succeeds; the published `.gitignore` is advisory.
#[test]
fn private_mode_can_be_re_entered_after_publish() {
    let repo = seeded_repo("reenter");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);
    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);

    let (out, err, again) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(again, "re-entering private mode after publish must work: {out} {err}");
    assert!(
        out.contains("exposed_advisory") || out.contains("warnings"),
        "the shared .gitignore should be reported as advisory: {out}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// Publish must not delete the user's own exclude lines that merely END with one
/// of our filenames — unrecoverable data loss in the file we promise to preserve.
#[test]
fn publish_keeps_user_paths_that_share_our_filenames() {
    let repo = seeded_repo("suffix");
    let (_, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(ok);

    let path = repo.join(".git/info/exclude");
    let content = std::fs::read_to_string(&path).unwrap();
    // Park the user's own same-named paths INSIDE our fenced block.
    let injected = content.replace(
        "/whetstone/\n",
        "/whetstone/\n/tools/legacy/.mcp.json\n/vendor/x/.githooks/post-merge\n/my/own/whetstone/\nKEEP-ME/\n",
    );
    std::fs::write(&path, injected).unwrap();

    let (_, _, pub_ok) = run_wh(&repo, &["publish", "--json"]);
    assert!(pub_ok);
    let after = std::fs::read_to_string(&path).unwrap();
    for keep in [
        "/tools/legacy/.mcp.json",
        "/vendor/x/.githooks/post-merge",
        "/my/own/whetstone/",
        "KEEP-ME/",
    ] {
        assert!(after.contains(keep), "{keep} must survive publish: {after}");
    }
    assert!(
        !after.contains("whetstone private mode"),
        "our fence must be gone: {after}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// An orphaned lock (no live holder) is reclaimed immediately, not after 30s of
/// "another `wh` process is updating it".
#[test]
fn orphaned_lock_is_reclaimed_immediately() {
    let repo = seeded_repo("lock");
    let lock = repo.join(".git/info/exclude.wh-lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    // A dead pid: PID 1 is alive, so use a very high one that cannot exist.
    std::fs::write(&lock, "4294967290").unwrap();

    let start = std::time::Instant::now();
    let (out, err, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    let elapsed = start.elapsed();
    assert!(ok, "an orphaned lock must not block: {out} {err}");
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "reclaim must be immediate, took {elapsed:?}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// `wh status` re-checks the promise, so a teammate's `.gitignore` negation
/// arriving via `git pull` does not go unnoticed.
#[test]
fn status_rechecks_private_mode_and_warns_when_broken() {
    let repo = seeded_repo("recheck");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);

    let (clean, _, clean_ok) = run_wh(&repo, &["status", "--json"]);
    assert!(clean_ok);
    assert!(
        !clean.contains("NO LONGER in effect"),
        "a healthy private repo must not warn: {clean}"
    );

    // A teammate re-includes our artifacts.
    std::fs::write(
        repo.join(".gitignore"),
        "node_modules/\n!.mcp.json\n!.claude/settings.json\n",
    )
    .unwrap();
    git_ok(&repo, &["add", ".gitignore"]);
    git_ok(&repo, &["commit", "-q", "-m", "teammate change"]);

    let (broken, _, broken_ok) = run_wh(&repo, &["status", "--json"]);
    assert!(broken_ok, "status stays a read-only health command");
    assert!(
        broken.contains("NO LONGER in effect"),
        "status must warn that private mode broke: {broken}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

// ── round-11 regressions ──

/// `is_private` must fail SAFE. An unparseable whetstone.yaml used to read as
/// "public", disengaging every tracked-file guard so `wh init --hooks` modified a
/// teammate's committed settings.json while reporting success.
#[test]
fn unparseable_marker_file_does_not_disengage_tracked_guards() {
    let repo = seeded_repo("failsafe");
    std::fs::create_dir_all(repo.join(".claude")).unwrap();
    let settings = "{\n  \"hooks\": {\n    \"PostToolUse\": [{\"matcher\": \"Edit\", \"hooks\": [{\"type\": \"command\", \"command\": \"team-hook.sh\"}]}]\n  }\n}\n";
    std::fs::write(repo.join(".claude/settings.json"), settings).unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "team hook"]);

    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);
    assert_eq!(git_status_porcelain(&repo), "");

    // The user mangles the config they are documented to hand-edit.
    std::fs::write(
        repo.join("whetstone/whetstone.yaml"),
        "setup:\n  private: true\n\tbroken: 1\n",
    )
    .unwrap();

    let (out, _, hooks_ok) = run_wh(&repo, &["init", "--hooks", "--json"]);
    assert!(hooks_ok, "should still succeed: {out}");
    assert_eq!(
        std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap(),
        settings,
        "an unreadable marker must NOT let us modify a tracked file"
    );
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "no tracked file may be modified: {out}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// A refused enable must leave no footprint at all — not even `?? whetstone/`.
#[test]
fn refused_enable_leaves_no_whetstone_dir() {
    let repo = seeded_repo("nodir");
    // UNTRACKED: a committed workflow is legitimately public and no leak at all.
    // An uncommitted one is a Whetstone artifact git can see, so enable refuses.
    let (_, _, ci_ok) = run_wh(&repo, &["init", "--ci", "--json"]);
    assert!(ci_ok);

    let (out, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(!ok, "must refuse with an inherently-shared workflow: {out}");
    assert!(
        !repo.join("whetstone").exists(),
        "a refusal must not leave whetstone/ behind: {out}"
    );
    // The workflow the user created is the ONLY thing visible — nothing of ours.
    let status = git_status_porcelain(&repo);
    assert!(
        status.lines().all(|l| l.contains(".github")),
        "a refusal must add no footprint of its own, got: {status:?}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// Concurrent enables across packages must not lose a block. This is the
/// round-11 MAJOR: a reclaimed live lock let two writers interleave the
/// read-modify-write and one block vanished.
#[test]
fn concurrent_enables_never_lose_a_block() {
    let repo = seeded_repo("concurrent");
    let n = 8;
    for i in 0..n {
        let pkg = repo.join(format!("p{i}"));
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            format!("{{\"name\":\"p{i}\",\"dependencies\":{{\"react\":\"^18.2.0\"}}}}\n"),
        )
        .unwrap();
    }
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "packages"]);

    let mut handles = Vec::new();
    for i in 0..n {
        let dir = repo.join(format!("p{i}"));
        handles.push(std::thread::spawn(move || {
            Command::new(whetstone_bin())
                .args(["init", "--private", "--json"])
                .current_dir(&dir)
                .env("WHETSTONE_NO_TUI", "1")
                .output()
                .expect("run whetstone")
                .status
                .success()
        }));
    }
    for h in handles {
        assert!(h.join().unwrap(), "every concurrent enable must succeed");
    }

    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    let blocks = exclude
        .lines()
        .filter(|l| l.starts_with("# >>> whetstone private mode"))
        .count();
    assert_eq!(blocks, n, "every package's block must survive: {exclude}");
    assert_eq!(
        git_status_porcelain(&repo),
        "",
        "no package may be left exposed"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// Concurrent publish + enable must converge: publish's removal cannot be lost.
#[test]
fn concurrent_publish_and_enable_both_land() {
    let repo = seeded_repo("pubrace");
    for i in 0..4 {
        let pkg = repo.join(format!("q{i}"));
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            format!("{{\"name\":\"q{i}\",\"dependencies\":{{\"react\":\"^18.2.0\"}}}}\n"),
        )
        .unwrap();
    }
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "packages"]);
    for i in 0..4 {
        let (_, _, ok) = run_wh(&repo.join(format!("q{i}")), &["init", "--private", "--json"]);
        assert!(ok);
    }

    // q0 publishes while q1..q3 re-enable concurrently.
    let mut handles = Vec::new();
    for i in 0..4 {
        let dir = repo.join(format!("q{i}"));
        let args: Vec<&str> = if i == 0 {
            vec!["publish", "--json"]
        } else {
            vec!["init", "--private", "--json"]
        };
        handles.push(std::thread::spawn(move || {
            Command::new(whetstone_bin())
                .args(&args)
                .current_dir(&dir)
                .env("WHETSTONE_NO_TUI", "1")
                .output()
                .expect("run whetstone")
                .status
                .success()
        }));
    }
    for h in handles {
        assert!(h.join().unwrap(), "every concurrent op must succeed");
    }

    let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    assert!(
        !exclude.contains("[q0]"),
        "publish's removal must not be lost: {exclude}"
    );
    for i in 1..4 {
        assert!(
            exclude.contains(&format!("[q{i}]")),
            "sibling q{i} must stay hidden: {exclude}"
        );
    }
    std::fs::remove_dir_all(&repo).ok();
}

/// A lock held by a LIVE process must be waited for, never stolen.
#[test]
fn a_live_holders_lock_is_not_stolen() {
    let repo = seeded_repo("livelock");
    let lock = repo.join(".git/info/exclude.wh-lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    // Our own pid is definitionally alive.
    std::fs::write(&lock, std::process::id().to_string()).unwrap();

    let (out, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
    assert!(!ok, "must not steal a live holder's lock: {out}");
    assert!(out.contains("another"), "should report contention: {out}");
    std::fs::remove_file(&lock).ok();
    std::fs::remove_dir_all(&repo).ok();
}

/// A lock left as a directory or a dangling symlink must not brick private mode.
#[test]
fn pathological_lock_files_do_not_brick_private_mode() {
    for kind in ["dir", "symlink"] {
        let repo = seeded_repo("badlock");
        let lock = repo.join(".git/info/exclude.wh-lock");
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        if kind == "dir" {
            std::fs::create_dir(&lock).unwrap();
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink("/nonexistent-whetstone-target", &lock).unwrap();
        }
        // Age it past the grace period.
        let (out, _, ok) = run_wh(&repo, &["init", "--private", "--json"]);
        // Either it recovers, or it fails with a clear message — never a
        // permanent brick with a misleading cause.
        assert!(
            ok || out.contains("another") || out.contains("lock"),
            "{kind} lock must give an actionable outcome: {out}"
        );
        std::fs::remove_dir_all(&repo).ok();
    }
}

/// The SessionStart hook must actually parse `wh status --json` (pretty-printed)
/// and surface a broken private mode. It was a permanent no-op.
#[test]
fn session_hook_surfaces_broken_private_mode() {
    let repo = seeded_repo("hookparse");
    let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
    assert!(ok);

    let hook = repo.join(".claude/whetstone-session-hook.sh");
    assert!(hook.exists(), "the session hook should be installed");

    // Break private mode the way a teammate would.
    std::fs::write(
        repo.join(".gitignore"),
        "node_modules/\n!.mcp.json\n!.claude/settings.json\n",
    )
    .unwrap();
    git_ok(&repo, &["add", ".gitignore"]);
    git_ok(&repo, &["commit", "-q", "-m", "teammate negation"]);

    let wh_dir = whetstone_bin().parent().unwrap().to_path_buf();
    let path = format!(
        "{}:{}",
        wh_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // The hook calls `wh`; the built binary is named `whetstone`, so expose it
    // under the name the hook expects.
    let shim = wh_dir.join("wh");
    if !shim.exists() {
        let _ = std::fs::hard_link(whetstone_bin(), &shim);
    }

    let out = Command::new("sh")
        .arg(&hook)
        .current_dir(&repo)
        .env("PATH", &path)
        .env("WHETSTONE_NO_TUI", "1")
        .output()
        .expect("run hook");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("NO LONGER in effect"),
        "the hook must surface the broken promise in-session, got: {combined:?}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// `is_private` must fail SAFE for every config shape it cannot understand — not
/// just unparseable YAML. A list, a bare scalar, an empty file, `setup: null`, or
/// a non-bool `private` all used to read as "public" and let us modify a tracked
/// file. A well-formed public config must still be public.
#[test]
fn every_unintelligible_marker_shape_fails_safe() {
    let malformed = [
        ("unparseable", "setup:\n  private: true\n\tbad: 1\n"),
        ("list", "- a\n- b\n"),
        ("scalar", "hello\n"),
        ("empty", ""),
        ("null_setup", "setup: null\n"),
        ("string_bool", "setup:\n  private: \"yes\"\n"),
    ];
    let settings = "{\n  \"hooks\": {\n    \"PostToolUse\": [{\"matcher\": \"Edit\", \"hooks\": [{\"type\": \"command\", \"command\": \"team.sh\"}]}]\n  }\n}\n";

    for (name, body) in malformed {
        let repo = seeded_repo("shape");
        std::fs::create_dir_all(repo.join(".claude")).unwrap();
        std::fs::write(repo.join(".claude/settings.json"), settings).unwrap();
        git_ok(&repo, &["add", "."]);
        git_ok(&repo, &["commit", "-q", "-m", "team settings"]);
        let (_, _, ok) = run_wh(&repo, &["init", "--claude", "--private", "--json"]);
        assert!(ok);

        std::fs::write(repo.join("whetstone/whetstone.yaml"), body).unwrap();
        let (out, _, _) = run_wh(&repo, &["init", "--hooks", "--json"]);
        assert_eq!(
            std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap(),
            settings,
            "{name}: an unintelligible marker must not let us modify a tracked file: {out}"
        );
        std::fs::remove_dir_all(&repo).ok();
    }

    // ...but a well-formed PUBLIC config must still be public, or every ordinary
    // project would silently stop wiring itself up.
    let repo = seeded_repo("publicshape");
    std::fs::create_dir_all(repo.join(".claude")).unwrap();
    std::fs::write(repo.join(".claude/settings.json"), settings).unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "team settings"]);
    std::fs::create_dir_all(repo.join("whetstone")).unwrap();
    std::fs::write(repo.join("whetstone/whetstone.yaml"), "version: 1\n").unwrap();

    let (out, _, ok) = run_wh(&repo, &["init", "--hooks", "--json"]);
    assert!(ok, "{out}");
    assert_ne!(
        std::fs::read_to_string(repo.join(".claude/settings.json")).unwrap(),
        settings,
        "a well-formed public config must still get its hooks written: {out}"
    );
    std::fs::remove_dir_all(&repo).ok();
}
