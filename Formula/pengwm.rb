class Pengwm < Formula
  desc "Tiling window manager for macOS (Accessibility & Core Graphics, no SIP changes)"
  homepage "https://github.com/Penqguin/PengWM"
  version "0.5"
  license "MIT"

  # NOTE TO MAINTAINERS: after tagging a release, update the four occurrences
  # of the tag below (version + both per-arch URLs embed it) and the two
  # sha256 lines, using the GitHub release assets:
  #   pengwm-v<VERSION>-aarch64-apple-darwin.tar.gz.sha256
  #   pengwm-v<VERSION>-x86_64-apple-darwin.tar.gz.sha256

  on_macos do
    on_arm do
      url "https://github.com/Penqguin/PengWM/releases/download/v0.5/pengwm-v0.5-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_SHA256"
    end
    on_intel do
      url "https://github.com/Penqguin/PengWM/releases/download/v0.5/pengwm-v0.5-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_SHA256"
    end
  end

  def install
    bin.install "pengwm", "pengwm-bar", "pengwm-menubar"
  end

  def caveats
    <<~EOS
      PengWM needs Accessibility permissions:
        System Settings → Privacy & Security → Accessibility → add #{opt_bin}/pengwm
      And: Desktop & Dock → turn on "Displays have separate Spaces", then:
        pengwm
      Note: Homebrew installs the binaries but not the launchd LaunchAgent;
      run `pengwm` manually (or use ./install.sh from a source checkout for
      the auto-start-at-login setup).
    EOS
  end

  test do
    assert_match "Usage", shell_output("#{bin}/pengwm --help 2>&1", 1)
  end
end
