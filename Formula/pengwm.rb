class Pengwm < Formula
  desc "Tiling window manager for macOS (Accessibility & Core Graphics, no SIP changes)"
  homepage "https://github.com/Penqguin/PengWM"
  version "0.5.0"
  license "MIT"

  # NOTE TO MAINTAINERS: after tagging a release, update the four occurrences
  # of the tag below (version + both per-arch URLs embed it) and the two
  # sha256 lines, using the GitHub release assets:
  #   pengwm-v<VERSION>-aarch64-apple-darwin.tar.gz.sha256
  #   pengwm-v<VERSION>-x86_64-apple-darwin.tar.gz.sha256

  on_macos do
    on_arm do
      url "https://github.com/Penqguin/PengWM/releases/download/v0.5.0/pengwm-v0.5.0-aarch64-apple-darwin.tar.gz"
      sha256 "07dcd41b4df25b06bd071ccc3bdf0a9aa1bbbe5e9a56c37500039c424a8d7a53"
    end
    on_intel do
      url "https://github.com/Penqguin/PengWM/releases/download/v0.5.0/pengwm-v0.5.0-x86_64-apple-darwin.tar.gz"
      sha256 "4af267d0a45e8b17be8d55c58a6bbc6dc3d0e6a92cd95b2831dc3dbb174bc66c"
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
