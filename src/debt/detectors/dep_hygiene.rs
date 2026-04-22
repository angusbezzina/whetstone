//! Dependency hygiene: imports that no manifest declares, and declared
//! packages that nothing imports. Runs on the detect-deps output plus a
//! language-specific import scan over the source inventory.

use anyhow::Result;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use crate::debt::types::{Category, Confidence, Evidence, Finding, Location, SourceInventory};
use crate::detect::detect_deps;

// Packages that should not be flagged as "unused" — they are consumed by
// tooling rather than by `import` statements. Keep this list tiny and
// only add entries with a clear rationale.
const PYTHON_TOOLING_WHITELIST: &[&str] = &[
    "pytest",
    "pytest-cov",
    "pytest-asyncio",
    "ruff",
    "black",
    "isort",
    "mypy",
    "pyright",
    "coverage",
    "tox",
    "pre-commit",
    "build",
    "twine",
    "setuptools",
    "wheel",
    "hatchling",
    "poetry-core",
    "uv",
];

const TS_TOOLING_WHITELIST: &[&str] = &[
    "typescript",
    "tsx",
    "ts-node",
    "eslint",
    "prettier",
    "vitest",
    "jest",
    "@types/node",
    "@types/jest",
    "@biomejs/biome",
    "rimraf",
    "concurrently",
    "npm-run-all",
];

const RUST_TOOLING_WHITELIST: &[&str] = &[];

// Python modules that ship under a different import name than their pypi name.
fn python_import_to_pkg(import_name: &str) -> &str {
    match import_name {
        "PIL" => "Pillow",
        "cv2" => "opencv-python",
        "sklearn" => "scikit-learn",
        "yaml" => "pyyaml",
        "bs4" => "beautifulsoup4",
        "dateutil" => "python-dateutil",
        "jwt" => "pyjwt",
        "dotenv" => "python-dotenv",
        "toml" => "toml",
        "magic" => "python-magic",
        other => other,
    }
}

pub fn run(project_dir: &Path, sources: &SourceInventory) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    // Reuse the existing detector so we don't re-parse manifests here.
    let deps = match detect_deps(project_dir, false, &[], &[], false) {
        Ok(v) => v,
        Err(_) => return Ok(findings),
    };

    let mut declared_python: BTreeMap<String, String> = BTreeMap::new();
    let mut declared_ts: BTreeMap<String, String> = BTreeMap::new();
    let mut declared_rust: BTreeMap<String, String> = BTreeMap::new();

    if let Some(arr) = deps.get("dependencies").and_then(|v| v.as_array()) {
        for d in arr {
            let name = d.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let lang = d.get("language").and_then(|v| v.as_str()).unwrap_or("");
            let version = d.get("version").and_then(|v| v.as_str()).unwrap_or("*");
            let dev = d.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);
            if dev {
                // Skip dev-only deps — they are often tooling-consumed.
                continue;
            }
            let entry = format!("{name} = {version}");
            match lang {
                "python" => {
                    declared_python.insert(name.to_lowercase(), entry);
                }
                "typescript" => {
                    declared_ts.insert(name.to_string(), entry);
                }
                "rust" => {
                    declared_rust.insert(name.to_string(), entry);
                }
                _ => {}
            }
        }
    }

    let imported_python = scan_python_imports(sources);
    let imported_ts = scan_ts_imports(sources);
    let imported_rust = scan_rust_uses(sources);

    // --- unused declared ---
    for (name, snippet) in &declared_python {
        if PYTHON_TOOLING_WHITELIST.contains(&name.as_str()) {
            continue;
        }
        let mapped: Vec<&str> = imported_python
            .iter()
            .map(|i| python_import_to_pkg(i))
            .collect();
        let referenced = mapped
            .iter()
            .any(|m| m.eq_ignore_ascii_case(name) || m.replace('_', "-").eq_ignore_ascii_case(name));
        if !referenced {
            findings.push(Finding {
                category: Category::Dead,
                rule_id: "dead.unused_declared_deps".into(),
                title: format!("Unused declared dependency: {name} (python)"),
                confidence: Confidence::High,
                evidence_strength: 1.0,
                files: vec!["pyproject.toml".into()],
                evidence: Evidence::ManifestEntry {
                    snippet: snippet.clone(),
                    references: 0,
                    locations: vec![Location {
                        file: "pyproject.toml".into(),
                        line: None,
                    }],
                },
                next_action: format!(
                    "Remove `{name}` from pyproject.toml if truly unused, or add the missing import it guards."
                ),
            });
        }
    }

    for (name, snippet) in &declared_ts {
        if TS_TOOLING_WHITELIST.contains(&name.as_str()) {
            continue;
        }
        if name.starts_with("@types/") {
            continue; // type-only; consumed implicitly by tsc
        }
        let referenced = imported_ts.iter().any(|i| i == name || i.starts_with(&format!("{name}/")));
        if !referenced {
            findings.push(Finding {
                category: Category::Dead,
                rule_id: "dead.unused_declared_deps".into(),
                title: format!("Unused declared dependency: {name} (typescript)"),
                confidence: Confidence::High,
                evidence_strength: 1.0,
                files: vec!["package.json".into()],
                evidence: Evidence::ManifestEntry {
                    snippet: snippet.clone(),
                    references: 0,
                    locations: vec![Location {
                        file: "package.json".into(),
                        line: None,
                    }],
                },
                next_action: format!(
                    "Remove `{name}` from package.json if truly unused, or add the missing import it guards."
                ),
            });
        }
    }

    for (name, snippet) in &declared_rust {
        if RUST_TOOLING_WHITELIST.contains(&name.as_str()) {
            continue;
        }
        let normalized = name.replace('-', "_");
        let referenced = imported_rust.iter().any(|i| i == &normalized);
        if !referenced {
            findings.push(Finding {
                category: Category::Dead,
                rule_id: "dead.unused_declared_deps".into(),
                title: format!("Unused declared dependency: {name} (rust)"),
                confidence: Confidence::Medium, // Rust can reach crates via renames/prelude; be conservative.
                evidence_strength: 1.0,
                files: vec!["Cargo.toml".into()],
                evidence: Evidence::ManifestEntry {
                    snippet: snippet.clone(),
                    references: 0,
                    locations: vec![Location {
                        file: "Cargo.toml".into(),
                        line: None,
                    }],
                },
                next_action: format!(
                    "Remove `{name}` from Cargo.toml if truly unused. Check for `extern crate` aliases before deleting."
                ),
            });
        }
    }

    // --- undeclared imports ---
    let stdlib_py = python_stdlib_set();
    for imp in &imported_python {
        let top = imp.split('.').next().unwrap_or(imp);
        if stdlib_py.contains(top) {
            continue;
        }
        let pkg = python_import_to_pkg(top).to_lowercase();
        let declared = declared_python
            .keys()
            .any(|k| k == &pkg || k.replace('-', "_") == pkg.replace('-', "_"));
        if !declared {
            findings.push(Finding {
                category: Category::Deps,
                rule_id: "deps.undeclared_import".into(),
                title: format!("Undeclared python import: {top}"),
                confidence: Confidence::Medium,
                evidence_strength: 1.0,
                files: vec![],
                evidence: Evidence::ManifestEntry {
                    snippet: format!("import {top}"),
                    references: 1,
                    locations: vec![],
                },
                next_action: format!(
                    "Add `{pkg}` to pyproject.toml dependencies, or remove the stray import."
                ),
            });
        }
    }

    // Keep undeclared-import checks for TS/Rust for a later pass — their import
    // → package mapping is more involved (workspace paths, relative imports,
    // extern crate aliases). Ship python v1, iterate.

    Ok(dedup_findings(findings))
}

fn dedup_findings(items: Vec<Finding>) -> Vec<Finding> {
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut out = Vec::with_capacity(items.len());
    for f in items {
        let key = (f.rule_id.clone(), f.title.clone());
        if seen.insert(key) {
            out.push(f);
        }
    }
    out
}

fn scan_python_imports(sources: &SourceInventory) -> HashSet<String> {
    let import_re = Regex::new(r"(?m)^\s*(?:from\s+([a-zA-Z_][\w.]*)|import\s+([a-zA-Z_][\w.]*))").unwrap();
    let mut out = HashSet::new();
    for path in &sources.python {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for cap in import_re.captures_iter(&text) {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                let top = m.as_str().split('.').next().unwrap_or("").to_string();
                if !top.is_empty() {
                    out.insert(top);
                }
            }
        }
    }
    out
}

fn scan_ts_imports(sources: &SourceInventory) -> HashSet<String> {
    let import_re = Regex::new(
        r#"(?m)^\s*(?:import[^'"`]*['"`]([^'"`]+)['"`]|import\(['"`]([^'"`]+)['"`]\)|require\(['"`]([^'"`]+)['"`]\))"#,
    )
    .unwrap();
    let mut out = HashSet::new();
    for path in &sources.typescript {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for cap in import_re.captures_iter(&text) {
            let spec = cap
                .get(1)
                .or_else(|| cap.get(2))
                .or_else(|| cap.get(3))
                .map(|m| m.as_str())
                .unwrap_or("");
            if spec.is_empty() {
                continue;
            }
            if spec.starts_with('.') || spec.starts_with('/') {
                continue; // relative or absolute path — not a package
            }
            let pkg = if spec.starts_with('@') {
                // scoped: @scope/pkg[/sub...]
                let parts: Vec<&str> = spec.splitn(3, '/').collect();
                if parts.len() >= 2 {
                    format!("{}/{}", parts[0], parts[1])
                } else {
                    spec.to_string()
                }
            } else {
                // plain: pkg[/sub]
                spec.split('/').next().unwrap_or(spec).to_string()
            };
            out.insert(pkg);
        }
    }
    out
}

fn scan_rust_uses(sources: &SourceInventory) -> HashSet<String> {
    // For rust we want the top-level crate name per `use` / `extern crate`.
    let use_re = Regex::new(r"(?m)^\s*(?:pub\s+)?(?:use|extern\s+crate)\s+([a-zA-Z_][\w]*)").unwrap();
    let mut out = HashSet::new();
    for path in &sources.rust {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for cap in use_re.captures_iter(&text) {
            if let Some(m) = cap.get(1) {
                let top = m.as_str();
                // Skip obvious crate-local prefixes.
                if matches!(top, "crate" | "super" | "self") {
                    continue;
                }
                out.insert(top.to_string());
            }
        }
    }
    out
}

fn python_stdlib_set() -> HashSet<&'static str> {
    // Abbreviated CPython stdlib top-level names. Not exhaustive; missing
    // entries just downgrade an import from "stdlib" to "undeclared", which
    // then gets flagged — acceptable for v1 since false positives are
    // easy to add to tooling whitelists or the map above.
    [
        "abc",
        "argparse",
        "array",
        "ast",
        "asyncio",
        "base64",
        "binascii",
        "bisect",
        "builtins",
        "bz2",
        "calendar",
        "collections",
        "colorsys",
        "concurrent",
        "configparser",
        "contextlib",
        "copy",
        "csv",
        "ctypes",
        "dataclasses",
        "datetime",
        "decimal",
        "difflib",
        "dis",
        "email",
        "enum",
        "errno",
        "faulthandler",
        "fcntl",
        "filecmp",
        "fileinput",
        "fnmatch",
        "fractions",
        "functools",
        "gc",
        "getopt",
        "getpass",
        "gettext",
        "glob",
        "grp",
        "gzip",
        "hashlib",
        "heapq",
        "hmac",
        "html",
        "http",
        "imaplib",
        "imp",
        "importlib",
        "inspect",
        "io",
        "ipaddress",
        "itertools",
        "json",
        "keyword",
        "linecache",
        "locale",
        "logging",
        "lzma",
        "mailbox",
        "math",
        "mimetypes",
        "mmap",
        "multiprocessing",
        "netrc",
        "numbers",
        "operator",
        "os",
        "pathlib",
        "pickle",
        "pkgutil",
        "platform",
        "plistlib",
        "pprint",
        "profile",
        "pstats",
        "pty",
        "pwd",
        "queue",
        "random",
        "re",
        "readline",
        "reprlib",
        "resource",
        "secrets",
        "select",
        "selectors",
        "shelve",
        "shlex",
        "shutil",
        "signal",
        "site",
        "smtplib",
        "socket",
        "socketserver",
        "sqlite3",
        "ssl",
        "stat",
        "statistics",
        "string",
        "stringprep",
        "struct",
        "subprocess",
        "sys",
        "sysconfig",
        "syslog",
        "tarfile",
        "telnetlib",
        "tempfile",
        "termios",
        "textwrap",
        "threading",
        "time",
        "timeit",
        "tkinter",
        "token",
        "tokenize",
        "trace",
        "traceback",
        "tracemalloc",
        "types",
        "typing",
        "unicodedata",
        "unittest",
        "urllib",
        "uuid",
        "venv",
        "warnings",
        "weakref",
        "webbrowser",
        "wsgiref",
        "xml",
        "xmlrpc",
        "zipfile",
        "zipimport",
        "zlib",
        "zoneinfo",
        "__future__",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debt::source_walk;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn flags_declared_python_dep_with_no_import() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("pyproject.toml"),
            r#"
[project]
name = "x"
version = "0.1.0"
dependencies = ["requests", "neverused"]
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/app.py"), "import requests\nrequests.get('/')\n").unwrap();

        let inv = source_walk::collect(root);
        let findings = run(root, &inv).unwrap();
        let titles: Vec<&str> = findings.iter().map(|f| f.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.contains("neverused")),
            "expected neverused to be flagged, got: {titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t.contains("requests")),
            "requests is imported, should not be flagged: {titles:?}"
        );
    }

    #[test]
    fn ignores_tooling_whitelist_dep() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("pyproject.toml"),
            r#"
[project]
name = "x"
version = "0.1.0"
dependencies = ["pytest"]
"#,
        )
        .unwrap();
        let inv = source_walk::collect(root);
        let findings = run(root, &inv).unwrap();
        assert!(
            findings.iter().all(|f| !f.title.contains("pytest")),
            "pytest is tooling and should be whitelisted"
        );
    }

    #[test]
    fn maps_pillow_alias() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("pyproject.toml"),
            r#"
[project]
name = "x"
version = "0.1.0"
dependencies = ["Pillow"]
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/app.py"),
            "from PIL import Image\nImage.open('x')\n",
        )
        .unwrap();

        let inv = source_walk::collect(root);
        let findings = run(root, &inv).unwrap();
        assert!(
            findings.iter().all(|f| !f.title.contains("Pillow")),
            "PIL import should map to Pillow"
        );
    }
}
