# Homebrew formula for pylon.
#
# Install via the tap:
#   brew install pylonsync/tap/pylon
#
# After cutting a release, bump `version` and refresh the SHAs:
#   ./script/update-formula.sh   # (or update by hand from the .sha256 sidecars)
#
# The release workflow in .github/workflows/release.yml builds the four
# archives this formula expects.
class Pylon < Formula
  desc "Self-hostable, single-binary backend for web, mobile, and real-time apps"
  homepage "https://pylonsync.com"
  version "0.2.6"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/pylonsync/pylon-releases/releases/download/v#{version}/pylon-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "734434a56eea1f6e2e7dcefedb9f798d3d03086ef2e3b388e7dc7003259e779c"
    else
      url "https://github.com/pylonsync/pylon-releases/releases/download/v#{version}/pylon-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "583486a57ee90fa245e7ea74025aaa0b96019e95e5ae43704cc7dae6b2d50b60"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/pylonsync/pylon-releases/releases/download/v#{version}/pylon-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "738faea38229f40ea9b85e1f35e6c49e5540e7deeacefb05194608f17173473a"
    else
      url "https://github.com/pylonsync/pylon-releases/releases/download/v#{version}/pylon-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "158ac8d902774e7f385db5878320dce06519a41f4f7c115c00398917bfe2b355"
    end
  end

  def install
    bin.install "pylon"
  end

  test do
    assert_match(/pylon/, shell_output("#{bin}/pylon --version"))
  end
end
