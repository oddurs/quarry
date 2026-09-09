# Homebrew formula for quarry.
#
# Lives in a tap: https://github.com/oddurs/homebrew-tap
#   brew install oddurs/tap/quarry
#
# The sha256 values are filled in by scripts/release once a tag is built.
class Quarry < Formula
  desc "See every server running on this machine, and whose project it came from"
  homepage "https://oddurs.github.io/quarry"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/oddurs/quarry/releases/download/v#{version}/quarry-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_DARWIN_SHA"
    end
    on_intel do
      url "https://github.com/oddurs/quarry/releases/download/v#{version}/quarry-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_DARWIN_SHA"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/oddurs/quarry/releases/download/v#{version}/quarry-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_LINUX_SHA"
    end
    on_intel do
      url "https://github.com/oddurs/quarry/releases/download/v#{version}/quarry-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_X86_64_LINUX_SHA"
    end
  end

  def install
    bin.install "quarry"
  end

  test do
    assert_match "quarry #{version}", shell_output("#{bin}/quarry --version")
    # --doctor exits non-zero only when a fatal dependency is missing, which
    # cannot be asserted in a sandbox, so this just checks it runs.
    system bin/"quarry", "--help"
  end
end
