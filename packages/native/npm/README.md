# npm packages (vtype desktop)

`@ishizakahiroshi/vtype` is a small Node launcher. The real program comes from one of four
platform packages, which npm installs only on the matching OS and CPU (their `os` / `cpu` fields),
so no `postinstall` script runs:

| Package | Binary |
|---|---|
| `@ishizakahiroshi/vtype-win32-x64` | `bin/vtype.exe` |
| `@ishizakahiroshi/vtype-darwin-arm64` | `bin/vtype` |
| `@ishizakahiroshi/vtype-darwin-x64` | `bin/vtype` |
| `@ishizakahiroshi/vtype-linux-x64` | `bin/vtype` |

`vtype/bin/vtype.js` finds the platform package's binary (or, in this repository, the sibling
folder `vtype-<os>-<cpu>/bin/`) and runs it with the same arguments and exit code.

These folders are not part of the pnpm workspace (`pnpm-workspace.yaml` lists only
`packages/*`), and the `bin/` folders stay empty in git. Nothing here is published yet.

## Publishing (by hand, per release)

1. Set the same version in all five `package.json` files (the four platform packages and the
   `optionalDependencies` of `vtype/package.json`) as `packages/native/Cargo.toml`.
2. Download the binaries from the GitHub Release of that version:
   `vtype-<ver>-windows-x64.zip` gives `vtype.exe`; the macOS universal tar.gz gives one binary
   for both `darwin-arm64` and `darwin-x64`; `vtype-<ver>-linux-x64.tar.gz` gives the Linux one.
   Put each in the package's `bin/` folder (`chmod 755` for macOS and Linux).
3. Publish the platform packages first, then the launcher:

   ```sh
   cd packages/native/npm
   for p in vtype-win32-x64 vtype-darwin-arm64 vtype-darwin-x64 vtype-linux-x64 vtype; do
     (cd "$p" && npm publish --access public)
   done
   ```

4. Check: `npx @ishizakahiroshi/vtype --version` prints the version.
5. Empty the `bin/` folders of the platform packages again.

After installing (`npm i -g @ishizakahiroshi/vtype`), users run `vtype install` once to connect it
to the Chrome extension. Because the registration points at the binary inside `node_modules`, run
`vtype install` again after moving or reinstalling Node.
