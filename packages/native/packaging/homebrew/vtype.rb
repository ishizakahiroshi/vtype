# Homebrew formula for vtype desktop (child plan C9-C3). A template: the release workflow
# (.github/workflows/native-release.yml) replaces @@VERSION@@ and @@SHA256@@ and attaches the
# result to the draft release as vtype.rb. Copy that file into the tap; see README.md here.
class Vtype < Formula
  desc "Voice input into any app, with Google Chrome's speech recognition"
  homepage "https://github.com/ishizakahiroshi/vtype"
  url "https://github.com/ishizakahiroshi/vtype/releases/download/native-v@@VERSION@@/vtype-@@VERSION@@-macos-universal.tar.gz"
  version "@@VERSION@@"
  sha256 "@@SHA256@@"
  license "MIT"

  depends_on :macos

  def install
    bin.install "vtype"
  end

  # `brew services start vtype` runs the daemon at login.
  service do
    run [opt_bin/"vtype", "daemon"]
    keep_alive false
  end

  def caveats
    <<~EOS
      vtype needs Google Chrome: it starts Chrome in a profile of its own for
      the speech recognition. To start vtype at login, run once:
        vtype install

      Allow vtype in System Settings > Privacy & Security > Accessibility,
      so it can type into other apps. After an update, you may need to allow it again.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/vtype --version")
  end
end
