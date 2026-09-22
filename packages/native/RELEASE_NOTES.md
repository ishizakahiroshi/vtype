vtype desktop 0.1.0: voice input into any app on Windows, macOS and Linux. It needs Google Chrome:
it starts Chrome in a profile of its own and uses Chrome's speech recognition, so what you say is
sent to Google by Chrome. The browser extension is not needed. This program itself sends nothing
over the network.

How to install it and use it the first time:
https://github.com/ishizakahiroshi/vtype#desktop

| File | For |
|---|---|
| `vtype-0.1.0-windows-x64.zip` | Windows 10 / 11 (x64). Unzip, then run `vtype install` to start it with Windows |
| `vtype-0.1.0.msix` | The Microsoft Store package (unsigned; the Store signs it) |
| `vtype-0.1.0-macos-universal.tar.gz` | macOS (Apple silicon and Intel). The Homebrew formula uses it |
| `vtype_0.1.0_amd64.deb` | Debian / Ubuntu (x64). Starts with the desktop session for every user |
| `vtype-0.1.0-linux-x64.tar.gz` | Other Linux distributions (x64). Run `vtype install` to start it with the session |
| `vtype.rb` | The Homebrew formula for this release |
| `THIRD_PARTY_NOTICES.txt` | Licences of the open-source crates inside |
| `SHA256SUMS.txt` | Checksums of the files above |

The binaries are not signed: Windows SmartScreen and macOS may ask you to confirm the first time.
On macOS, allow vtype under System Settings > Privacy & Security > Accessibility.

Changes: see [CHANGELOG.md](https://github.com/ishizakahiroshi/vtype/blob/main/CHANGELOG.md).
