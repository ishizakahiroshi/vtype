# vtype desktop (`packages/native`)

One Rust program, `vtype`, for Windows, macOS and Linux. It types what the vtype Chrome extension
recognises into the app in front. What it does for users is in the repository's
[README](../../README.md#desktop); this file is for building and releasing it.

This crate is not part of the pnpm workspace. The strings users read come from the extension's
`_locales/<code>/messages.json` (keys starting with `native_`), which `build.rs` reads at build
time.

## Layout

| Path | What it is |
|---|---|
| `src/daemon.rs` | The resident app's decisions (start, stop, type, notify), with no system calls. Tested with a fake platform |
| `src/platform/` | The `Platform` trait and one folder per system (`windows/`, `macos/`, `linux/`) |
| `src/cli.rs` | The subcommands (`daemon`, `toggle`, `mode`, `install`, …) |
| `src/speech_host.rs`, `src/chrome_launch.rs` | The speech page on 127.0.0.1 and the Chrome (a profile of vtype's own) that runs it |
| `src/ipc.rs` | The local socket / pipe between the command line and the daemon |
| `src/install.rs` | `vtype install` / `uninstall`: starting with the OS |
| `packaging/msix/` | The MSIX manifest (Microsoft Store). Built by `scripts/release/build-msix.ps1` |
| `packaging/deb/` | Files the `.deb` installs (see `[package.metadata.deb]` in `Cargo.toml`) |
| `packaging/homebrew/` | The Homebrew formula template and how to set up the tap |
| `npm/` | The npm launcher and the four per-platform packages |
| `about.toml`, `about.hbs` | `cargo about` settings for `THIRD_PARTY_NOTICES.txt` |

## Development

The Rust version is pinned in `rust-toolchain.toml`.

Run `pnpm install` and `pnpm -r build` at the repository root first: `build.rs` embeds the speech
page from `packages/extension/dist-desktop/`, and the build stops with that hint when it is missing.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

CI (the `rust` job in `.github/workflows/ci.yml`) runs these on Windows, macOS and Linux. Linux needs
`libgtk-3-dev libxdo-dev libayatana-appindicator3-dev libxkbcommon-dev`.

## Releasing

Nothing is published automatically. The steps, in order:

1. Set the version in `Cargo.toml` (and `Cargo.lock` follows with `cargo build`), in the five
   `npm/*/package.json` files, and in `RELEASE_NOTES.md`. Commit.
2. Tag `native-v<version>` and push the tag. `.github/workflows/native-release.yml` checks that the
   tag matches `Cargo.toml`, builds all three systems and creates a **draft** GitHub Release with:
   `vtype-<ver>-windows-x64.zip`, `vtype-<ver>.msix` (unsigned), `vtype-<ver>-macos-universal.tar.gz`,
   `vtype_<ver>_amd64.deb`, `vtype-<ver>-linux-x64.tar.gz`, `THIRD_PARTY_NOTICES.txt`,
   `SHA256SUMS.txt` and `vtype.rb`. Running the workflow by hand (`gh workflow run
   native-release.yml`) is a dry run that only leaves these as workflow artifacts.
3. Check the draft, then press **Publish release**.
4. Microsoft Store: reserve the app in Partner Center, then build the MSIX with the real identity:

   ```powershell
   ./scripts/release/build-msix.ps1 -IdentityName <Package/Identity/Name> `
     -Publisher <Package/Identity/Publisher> -PublisherDisplayName <Publisher display name>
   ```

   The Store signs it. The only restricted capability is `runFullTrust` (a desktop program).
5. Homebrew: see `packaging/homebrew/README.md`.
6. npm: see `npm/README.md`.

The binaries are not signed (no Apple Developer Program; the Store signs the MSIX).
