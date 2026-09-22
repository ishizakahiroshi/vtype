vtype desktop 0.1.0: type what the vtype Chrome extension recognises into any app on Windows,
macOS and Linux. Chrome still does the speech recognition; this program sends nothing over the
network.

How to install it and connect it to the extension:
https://github.com/ishizakahiroshi/vtype#desktop

| File | For |
|---|---|
| `vtype-0.1.0-windows-x64.zip` | Windows 10 / 11 (x64). Unzip, then run `vtype install` |
| `vtype-0.1.0.msix` | The Microsoft Store package (unsigned; the Store signs it) |
| `vtype-0.1.0-macos-universal.tar.gz` | macOS (Apple silicon and Intel). The Homebrew formula uses it |
| `vtype_0.1.0_amd64.deb` | Debian / Ubuntu (x64). Registers itself with Chrome and Chromium for every user |
| `vtype-0.1.0-linux-x64.tar.gz` | Other Linux distributions (x64). Run `vtype install` |
| `vtype.rb` | The Homebrew formula for this release |
| `THIRD_PARTY_NOTICES.txt` | Licences of the open-source crates inside |
| `SHA256SUMS.txt` | Checksums of the files above |

The binaries are not signed: Windows SmartScreen and macOS may ask you to confirm the first time.
On macOS, allow vtype under System Settings > Privacy & Security > Accessibility.

Changes: see [CHANGELOG.md](https://github.com/ishizakahiroshi/vtype/blob/main/CHANGELOG.md).
