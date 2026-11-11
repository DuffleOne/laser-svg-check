# Laser Check

Illustrator sometimes exports SVGs with lines stacked on top of each other, so the laser cuts the same line two or three times. Laser Check watches a folder (Downloads by default) for new SVGs, pops up an alert when it finds doubled lines, and can write a fixed copy with every line cut once.

Open the app on its own to drop SVGs or folders onto it and review a batch.

## Install

Needs Rust (`brew install rustup && rustup default stable`). Then:

```sh
./scripts/install.sh
```

That builds the app into `~/Applications/Laser Check.app` and adds a login agent that runs the watcher. The first time it reads Downloads, macOS asks for permission. Run it again after pulling changes; rebuilding re-signs the app, so macOS may ask again.

`./scripts/install.sh --no-agent` installs just the app.

To stop the watcher:

```sh
launchctl bootout gui/$(id -u)/one.duffle.laser-check
```

## How it decides

Every path, polygon, rect, circle and line is flattened (with transforms applied) and compared against everything nearby. Two lines count as doubled where they run alongside each other within `tolerance_mm` for at least `min_overlap_mm`, measured along the whole path. Crossings, touching curves and sharp tips don't count.

The fix keeps the first copy in document order and cuts the later ones out of their paths, joining what's left back up. Untouched elements, layers, colours and ids stay byte for byte. It writes `name-fixed.svg` next to the original and never edits the original.

## Repeat warnings

Files are remembered by content hash in `~/Library/Application Support/laser-check/seen.json`. Clean files are never checked again. A file it already warned about is skipped until `rewarn_days` have passed, then it warns again. Fixed copies are marked clean as they're written, so the watcher doesn't trip over its own output.

## Config

`~/.config/laser-check/config.toml`, created on first run:

```toml
watch_dirs = ["~/Downloads"]
rewarn_days = 3.0
tolerance_mm = 0.05
min_overlap_mm = 2.0
alert_sound = "Sosumi"   # any name from /System/Library/Sounds, or "" for none
```

Thresholds are re-read on every check. Changing `watch_dirs` needs a restart of the watcher (`launchctl kickstart -k gui/$(id -u)/one.duffle.laser-check`).

## From a terminal

```sh
laser-check check <files or folders>   # report
laser-check fix <files or folders>     # write -fixed copies
laser-check verify a.svg a-fixed.svg   # confirm the fix didn't lose any line
```

The binary lives at `~/Applications/Laser Check.app/Contents/MacOS/laser-check`.
