cask "pengwm" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.5.1"
  # After tagging a release: update the three occurrences of the version
  # above/below (version + per-arch URLs embed it) and the two sha256
  # lines, using the release asset sidecars:
  #   pengwm-app-v<VERSION>-aarch64-apple-darwin.tar.gz.sha256 → arm
  #   pengwm-app-v<VERSION>-x86_64-apple-darwin.tar.gz.sha256  → intel
  sha256 arm:   "626dd86fd8b555dfcc506f5d7222d1c32564ef4b1d551dde77ce6793d5530291",
         intel: "97f2a3005439007881b8d2848933387e6c8e48454436f9d400d713742e9d1124"

  url "https://github.com/Penqguin/PengWM/releases/download/v#{version}/pengwm-app-v#{version}-#{arch}-apple-darwin.tar.gz"
  name "PengWM"
  desc "Tiling window manager for macOS (Accessibility & Core Graphics, no SIP changes)"
  homepage "https://github.com/Penqguin/PengWM"

  depends_on macos: :sonoma

  app "PengWM.app"
  binary "#{appdir}/PengWM.app/Contents/MacOS/pengwm"

  # Stop the daemon on uninstall. The LaunchAgent plist (if the user ran
  # install.sh) is torn down by the launchctl stanza; zap handles leftovers.
  uninstall launchctl: "com.pengwm.daemon",
            quit:      "com.pengwm.daemon"

  zap trash: [
    "~/.config/pengwm",
    "~/Library/LaunchAgents/com.pengwm.daemon.plist",
    "~/Library/Logs/pengwm.log",
  ]

  caveats <<~EOS
    PengWM needs Accessibility permissions:
      System Settings → Privacy & Security → Accessibility → add PengWM
    And: Desktop & Dock → turn on "Displays have separate Spaces"

    brew does not auto-start the daemon. Either enable it at login
    (install.sh --no-app handles the LaunchAgent for a non-brew install)
    or launch it manually at any time:
      open #{appdir}/PengWM.app  # or: pengwm
  EOS
end
