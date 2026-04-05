# Whetstone Homebrew formula

This directory holds the canonical Homebrew formula for Whetstone.

## Users: install Whetstone via Homebrew

Once the tap repository is published, install with:

```bash
brew install angusbezzina/tap/whetstone
```

If the tap is not yet published, use `install.sh` (see top-level README) or
install with `cargo install --git https://github.com/angusbezzina/whetstone`.

## Maintainers: publishing a new release

1. Create and push a tag (`git tag v0.2.0 && git push --tags`). The
   `.github/workflows/release.yml` workflow builds per-target binaries and
   generates `checksums-sha256.txt`.
2. Update `whetstone.rb` in this directory:
   - Bump `version`.
   - Copy each per-target sha256 out of the published `checksums-sha256.txt`
     and paste it into the matching `sha256` field.
3. Copy the updated `whetstone.rb` into the tap repository at
   `angusbezzina/homebrew-tap/Formula/whetstone.rb` and push. Homebrew picks
   up the new formula on the next `brew update`.

The tap repository is a minimal sibling repo. It only needs a `Formula/`
directory containing `whetstone.rb` — no additional scaffolding.

## Local verification

You can lint the formula without publishing by running:

```bash
brew audit --strict --online packaging/homebrew/whetstone.rb
brew install --build-from-source --formula packaging/homebrew/whetstone.rb
```

(These commands are optional; CI does not gate on them because Homebrew is
not available on every runner.)
