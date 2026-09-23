# Homebrew formula (vtype desktop)

`vtype.rb` is a template. Nothing here publishes it: the tap repository does not exist yet, and
creating it is a release step taken by hand.

## How the formula gets its values

The release workflow (`.github/workflows/native-release.yml`, on a `native-v*` tag) builds
`vtype-<version>-macos-universal.tar.gz`, puts its SHA-256 and the version into this template, and
attaches the result to the draft GitHub Release as `vtype.rb`. Check that `@@` no longer appears
in it.

## Setting up the tap (once)

1. Create the public repository `ishizakahiroshi/homebrew-tap` on GitHub (empty, MIT).
2. Add a `Formula/` folder to it.

## Each release

1. Publish the draft release first (the formula's URL points at its asset).
2. Download `vtype.rb` from the release and commit it as `Formula/vtype.rb` in the tap.
3. Check it on a Mac:

   ```sh
   brew install --build-from-source ishizakahiroshi/tap/vtype   # or: brew install ishizakahiroshi/tap/vtype
   brew test vtype
   brew audit --strict vtype
   ```

Users then install with `brew install ishizakahiroshi/tap/vtype` and start vtype. The first-run
screen asks whether to start it at login (ticked by default), and the settings page switches it
later; `vtype install` does the same from the command line. `brew services start vtype` also starts
it at login, but through Homebrew's own entry, which the settings page cannot see or switch off, so
use one or the other.

The binary is not signed or notarized (parent plan D28), so macOS may ask the user to confirm it the
first time, and the Accessibility permission may have to be given again after an update.
