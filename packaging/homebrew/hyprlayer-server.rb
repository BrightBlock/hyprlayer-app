# Formula, not a cask: this is a CLI-shaped binary, not an .app bundle.
# Lives in BrightBlock/homebrew-tap; @VERSION@/@SHA256@ are filled at release.
class HyprlayerServer < Formula
  desc "Hyprlayer runtime as a headless server, reached over a local socket or SSH"
  homepage "https://brightblock.ai/hyprlayer"
  version "@VERSION@"
  license :cannot_represent

  on_macos do
    on_arm do
      url "https://github.com/BrightBlock/hyprlayer-releases/releases/download/v#{version}/hyprlayer-server-aarch64-apple-darwin"
      sha256 "@SHA256_MACOS_ARM64@"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/BrightBlock/hyprlayer-releases/releases/download/v#{version}/hyprlayer-server-x86_64-unknown-linux-gnu"
      sha256 "@SHA256_LINUX_X86_64@"
    end
  end

  def install
    bin.install Dir["hyprlayer-server-*"].first => "hyprlayer-server"
  end

  # Deliberately no `service` block. ADR-0009 requires a LaunchAgent in the user's
  # session — `security find-generic-password` needs an unlocked login keychain,
  # which a brew-managed daemon does not have. Installing must also never start a
  # server: one server owns the profile, and the desktop starts its own.
  def caveats
    <<~EOS
      hyprlayer-server is installed but not running, by design.

      Run it directly:
        hyprlayer-server serve

      Or load it as a user agent (never a LaunchDaemon — the Keychain lookup
      needs an unlocked login keychain):
        cp #{opt_prefix}/ai.brightblock.hyprlayer.server.plist ~/Library/LaunchAgents/
        launchctl load ~/Library/LaunchAgents/ai.brightblock.hyprlayer.server.plist

      The desktop app ships its own copy and starts it for you; this package is
      for a machine you reach over SSH.
    EOS
  end

  test do
    assert_match "usage: hyprlayer-server", shell_output("#{bin}/hyprlayer-server bogus 2>&1", 2)
  end
end
