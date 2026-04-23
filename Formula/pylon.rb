# Homebrew formula for agentdb.
#
# To install from this tap:
#   brew tap ericc59/agentdb https://github.com/ericc59/agentdb
#   brew install agentdb
#
# Or directly from this file:
#   brew install --formula ./Formula/agentdb.rb
#
# After cutting a release, update `version`, the URLs, and the SHAs.
# The release workflow in .github/workflows/release.yml builds the four
# archives this formula expects.
class Agentdb < Formula
  desc "Self-hostable, single-binary backend for web, mobile, and real-time apps"
  homepage "https://github.com/ericc59/agentdb"
  version "0.1.0"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/ericc59/agentdb/releases/download/v#{version}/agentdb-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_AFTER_RELEASE"
    else
      url "https://github.com/ericc59/agentdb/releases/download/v#{version}/agentdb-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_AFTER_RELEASE"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/ericc59/agentdb/releases/download/v#{version}/agentdb-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256_AFTER_RELEASE"
    else
      url "https://github.com/ericc59/agentdb/releases/download/v#{version}/agentdb-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256_AFTER_RELEASE"
    end
  end

  def install
    bin.install "agentdb"
  end

  test do
    assert_match(/agentdb/, shell_output("#{bin}/agentdb version"))
  end
end
