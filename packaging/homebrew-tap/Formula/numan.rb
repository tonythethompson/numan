# typed: false
# frozen_string_literal: true

class Numan < Formula
  desc "Cross-platform package manager for Nushell"
  homepage "https://github.com/tonythethompson/numan"
  version "0.1.3"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "175a7449b97ca284107c9ee00f3a965a0534f5491ff2586866f0e018eb979581"
    end
    on_intel do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "ccaf668ed372de5a650cc5293c267fcf92bcbd545f647bc926deb8de5463ddaf"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "55a6747f12454f25976a3e919de8909eb0eb70136519abff061d283996d5f41f"
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
