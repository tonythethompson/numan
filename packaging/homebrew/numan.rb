# typed: false
# frozen_string_literal: true

# Homebrew formula for Numan — cross-platform Nushell package manager.
#
# Install without a tap (uses this file from the main repo):
#   brew install --formula https://raw.githubusercontent.com/tonythethompson/numan/master/packaging/homebrew/numan.rb
#
# Update version and sha256 values when cutting a release (see docs/PACKAGING.md).

class Numan < Formula
  desc "Cross-platform package manager for Nushell"
  homepage "https://github.com/tonythethompson/numan"
  version "0.1.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "72e3f8f1710f941228923927af2abafdc94c06b48074fa5031fec57003625f0e"
    end
    on_intel do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "72d582aaeca42a56ee16f03dc858a82b1b301fe5d194973fc6cb640d8e069e88"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "6fcb5cfd825f81050df57e0b51ed84873c7fcf88fabbf8084bd47636f7e74cb8"
    end
  end

  def install
    arch_dir = Dir["numan-*"].first
    odie "expected numan-* directory in archive" if arch_dir.nil?

    bin.install "#{arch_dir}/numan"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/numan --version")
  end
end
