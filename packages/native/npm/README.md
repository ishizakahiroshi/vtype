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
`packages/*`), and the `bin/` folders stay empty in git. The packages are on npm from 0.1.0.

## Publishing (by hand, per release)

The release workflow (`.github/workflows/native-release.yml`) packs all five on Linux, after its
release job and from the same zip and tar.gz files it attaches to the GitHub Release, and keeps them
as the run's `npm-packages` artifact. Do not pack them yourself on Windows: `npm pack` there writes
every file as 0644, and npm gives the executable bit back only to files listed in `bin` (the
platform packages list none), so the macOS and Linux binaries would not start after `npm i`.

1. Before tagging, set the same version in all five `package.json` files (the four platform
   packages and the `optionalDependencies` of `vtype/package.json`) as `packages/native/Cargo.toml`.
   The workflow's `npm packages` job fails when they differ.
2. After the tag's run has finished, download the packages from that run and check that they were
   packed from the released files (the zip and both tar.gz, so 3 lines):

   ```sh
   run=$(gh run list --workflow native-release.yml --branch native-v<ver> --limit 1 --json databaseId --jq '.[0].databaseId')
   gh run download "$run" -n npm-packages -D npm-packages
   cd npm-packages
   gh release download native-v<ver> -p SHA256SUMS.txt
   grep -Fxf SOURCE-SHA256SUMS.txt SHA256SUMS.txt | wc -l
   ```

   Artifacts expire (after 90 days by default), so publish soon after the release.
3. Publish the platform packages first, then the launcher:

   ```sh
   for p in vtype-win32-x64 vtype-darwin-arm64 vtype-darwin-x64 vtype-linux-x64 vtype; do
     npm publish "ishizakahiroshi-$p-<ver>.tgz" --access public
   done
   ```

4. Check: `npm view @ishizakahiroshi/vtype version` prints the version (it can take a minute or
   two to appear), and in an empty folder `npm i @ishizakahiroshi/vtype@<ver>` followed by
   `npx --no-install vtype --version` prints it too.

After installing (`npm i -g @ishizakahiroshi/vtype`), users start vtype. The first-run screen asks
whether to start it with the OS (ticked by default), and the settings page switches it later;
`vtype install` does the same from the command line (and, on GNOME, adds the shortcut). Google
Chrome must be installed; vtype starts it for the speech recognition. The sign-in entry points at
the binary inside `node_modules`. After moving or reinstalling Node, install the package again and
start vtype once (`vtype quit` first if the old copy is still running): when vtype starts, an entry
that starts another copy is pointed at the binary now running. It only rewrites an entry that is
there; it does not add one.
