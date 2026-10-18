//! Shared test support: a `bd` in front of the real one that answers the
//! private store's `bd init` by copying a store initialised once per test
//! run. Whetstone runs that init outside any Git repository, so the copy is
//! the whole effect; `bd init` inside a repository (the shareable store) and
//! every other call go to the real `bd`. A fresh `bd init` costs seconds per
//! test; tests/beads_store.rs keeps covering the real one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

const SHIM: &str = r#"#!/bin/sh
real='@REAL@'
templates='@TEMPLATES@'
if [ "$1" = init ] && ! git rev-parse --git-dir >/dev/null 2>&1; then
  key=$( { "$real" version; echo "$real $*"; } | cksum | cut -d' ' -f1)
  template="$templates/$key"
  target="${BEADS_DIR:-$PWD/.beads}"
  mkdir -p "$templates"
  if [ ! -f "$template/.beads/metadata.json" ]; then
    if mkdir "$template.lock" 2>/dev/null; then
      stage="$template.stage"
      rm -rf "$stage" && mkdir "$stage"
      (cd "$stage" && BEADS_DIR="$stage/.beads" GIT_CEILING_DIRECTORIES="$templates" "$real" "$@") \
        && mv "$stage" "$template"
      rm -rf "$stage" "$template.lock"
    else
      waited=0
      while [ ! -f "$template/.beads/metadata.json" ] && [ -d "$template.lock" ] && [ "$waited" -lt 120 ]; do
        sleep 1; waited=$((waited + 1))
      done
    fi
  fi
  if [ -f "$template/.beads/metadata.json" ]; then
    cp -R "$template/.beads" "$target" && exit 0
  fi
fi
exec "$real" "$@"
"#;

/// The real `bd` on the inherited `PATH`, skipping `skip`.
fn real_bd(skip: &Path) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir| dir != skip)
        .map(|dir| dir.join("bd"))
        .find(|candidate| candidate.is_file())
        .expect("bd on PATH")
}

/// A `PATH` whose `bd` copies a template store for the private store's init.
pub fn path_with_bd_template() -> OsString {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bd-template");
    let bin = root.join("bin");
    let shim = bin.join("bd");
    if !shim.is_file() {
        std::fs::create_dir_all(&bin).expect("bd template dir");
        let text = SHIM
            .replace("@REAL@", &real_bd(&bin).display().to_string())
            .replace("@TEMPLATES@", &root.join("templates").display().to_string());
        let staged = bin.join(format!("bd.{}", std::process::id()));
        std::fs::write(&staged, text).expect("bd shim");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .expect("bd shim mode");
        }
        std::fs::rename(&staged, &shim).expect("bd shim in place");
    }
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).expect("PATH")
}
