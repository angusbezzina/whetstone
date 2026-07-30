# Homebrew formula template for Whetstone.
#
# This file is the canonical source of truth for the Homebrew tap. To publish
# a new release:
#
#   1. Tag a release in angusbezzina/whetstone (e.g. v0.2.0); release.yml
#      will build per-target binaries and a checksums-sha256.txt file.
#   2. Update the `version` below and the four sha256 values with the
#      matching entries from the published checksums-sha256.txt.
#   3. Commit this file to your tap repository at
#      angusbezzina/homebrew-tap/Formula/whetstone.rb. The tap repo only
#      needs that single Formula directory — no extra scaffolding.
#
# Users then install with:
#
#   brew install angusbezzina/tap/whetstone
#
# The formula downloads the prebuilt release binary for the user's platform
# and performs a sha256 verification identical to install.sh.

class Whetstone < Formula
  desc "Whetstone sharpens the tools that write your code"
  homepage "https://github.com/angusbezzina/whetstone"
  version "0.12.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/angusbezzina/whetstone/releases/download/v#{version}/whetstone-aarch64-apple-darwin"
      sha256 "307c4edac1d036efe0e96177f82eb68e0e6c3e758a18f926a2be8f1de6368541"
    end
    on_intel do
      url "https://github.com/angusbezzina/whetstone/releases/download/v#{version}/whetstone-x86_64-apple-darwin"
      sha256 "bf6eaebbfde2b199321d6c12dbd4f45a79ea955422554942aa55198fc0a30f66"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/angusbezzina/whetstone/releases/download/v#{version}/whetstone-aarch64-unknown-linux-gnu"
      sha256 "754c7ae8c2d3c95f52d9d81256e62da572563534b989adf3e43f392c595b53fb"
    end
    on_intel do
      url "https://github.com/angusbezzina/whetstone/releases/download/v#{version}/whetstone-x86_64-unknown-linux-gnu"
      sha256 "9721cbb5cf239e43c7569ee9efa9c405c57ac17446d91f02557ebb32199844ca"
    end
  end

  def install
    # The release artifact is named whetstone-<target>; rename to plain `whetstone`.
    binary = Dir["whetstone-*"].first
    odie "No whetstone binary found in release archive" if binary.nil?
    bin.install binary => "whetstone"
  end

  test do
    assert_match "whetstone #{version}", shell_output("#{bin}/whetstone --version")
    assert_match "ci-check", shell_output("#{bin}/whetstone --help")
  end
end
