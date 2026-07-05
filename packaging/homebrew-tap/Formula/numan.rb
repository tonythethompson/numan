# typed: false
# frozen_string_literal: true

class Numan < Formula
  desc "Cross-platform package manager for Nushell"
  homepage "https://github.com/tonythethompson/numan"
  version "0.1.4"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "5f9e66268c0aa1953d643548ae6f9bd9c9a918ae74c004d6405637186438baae"
    end
    on_intel do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "45827829c0df923bf514772457ae34e4dfe9f4b9b966a3d28484e0a7a3ef7297"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/tonythethompson/numan/releases/download/v#{version}/numan-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1ed2a22d41bd5b767c5ad9017432631d97e73ea71465e2aebc58d55cad01ec92"
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
