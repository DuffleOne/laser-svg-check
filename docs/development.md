# Development

## Setup

```sh
brew install rustup && rustup default stable
cargo build --release
```

Rust 1.88 or later (the code uses let chains). The Xcode command line tools are enough: gpui is built with `runtime_shaders`, so its Metal shaders are compiled when the app starts and the Metal toolchain isn't needed.

## Running

```sh
cargo run --release                         # empty review window
cargo run --release -- examples             # review the example files
cargo run --release -- alert examples/duplicated-layer.svg
cargo run --release -- check examples
```

To try the watcher without touching your real settings or history, point `HOME` somewhere else:

```sh
mkdir -p /tmp/lc/.config/laser-check /tmp/lc/drop
echo 'watch_dirs = ["/tmp/lc/drop"]' > /tmp/lc/.config/laser-check/config.toml
HOME=/tmp/lc cargo run --release -- watch
# in another terminal
cp examples/honeycomb-coaster.svg /tmp/lc/drop/
```

## Layout

```
src/
  main.rs      command line entry point, and tests
  geom.rs      SVG → segments, with transforms and source byte ranges
  analyze.rs   flattening, spatial hash, doubled-stretch detection, fix plan
  fix.rs       rebuilds touched elements and splices them into the original text
  verify.rs    checks a fixed file still covers every original line
  render.rs    tiny-skia preview with the red highlights
  ui.rs        app setup, bundled icons and font, shared types
  ui/theme.rs  design tokens, icons, buttons, number formatting
  ui/review.rs review window: sidebar, preview with area markers, inspector
  ui/alert.rs  the watcher's alert, including fixing and verifying
  watch.rs     FSEvents watcher, and the short-lived `scan` process
  store.rs     content-hash history (seen.json)
  config.rs    config.toml
scripts/
  bundle.sh       build dist/Laser Check.app (--universal for releases)
  install.sh      bundle, then install-app.sh
  install-app.sh  install a built app + login agent; ships in the release zip
  Info.plist      app bundle template
  make-icon.swift renders assets/icon.svg into assets/AppIcon.icns
assets/        app icon, Lucide icons (icons/) and JetBrains Mono (fonts/)
examples/      synthetic SVGs used in the docs and CI
docs/          these docs and the README images
```

## Tests

```sh
cargo test --release
cargo clippy --release --all-targets -- -D warnings
cargo fmt --check
```

The unit tests in `main.rs` build small SVGs by hand: exact duplicates, a shared edge, crossing lines, touching circles and a duplicate under a transform. Each problem case is fixed and checked again. CI also runs the CLI against `examples/` and verifies a fix.

When changing the detection or the fix, run it over real exports and verify:

```sh
laser-check fix some/folder
for f in some/folder/*-fixed.svg; do laser-check verify "${f%-fixed.svg}.svg" "$f"; done
```

`laser-check preview file.svg out.png --area 3` is handy for seeing exactly what was flagged.

## gpui notes

- **Opening a window from a click handler deadlocks.** Making the new window key calls back into the window whose handler is still running. Wrap it in `cx.defer`. See `Alert::review`.
- **Closing the last window quits the app**, by design. So when swapping one window for another, open the new one before removing the old.
- The preview is drawn with tiny-skia on a background thread and shown as a `RenderImage`. gpui expects BGRA, so the channels are swapped. Old images are released with `cx.drop_image`.
- The alert uses `WindowKind::PopUp`, which floats above other apps' windows. It has no title bar, so macOS draws its corners and shadow; a `canvas` inside the card measures it and resizes the window to fit.
- A window on a Space you can't see stops redrawing, so screenshots and clicks there look stale. Test from a normal desktop Space.
- Colours come from the tokens in `ui/theme.rs`, which follow the design brief. Only dark mode has been designed; light mode keeps the older values.

## The icon

`assets/icon.svg` is the source. After editing it:

```sh
SET=$(swift scripts/make-icon.swift "$PWD") && iconutil -c icns "$SET" -o assets/AppIcon.icns
```

The banner in `docs/images/banner.svg` embeds a copy of the icon's artwork, so update that too.

## Screenshots

The README screenshots come from the example files, with a throwaway `HOME` so the alert sound stays off:

```sh
mkdir -p /tmp/shot/.config/laser-check && echo 'alert_sound = ""' > /tmp/shot/.config/laser-check/config.toml
HOME=/tmp/shot target/release/laser-check examples/honeycomb-coaster.svg examples/duplicated-layer.svg examples/clean.svg
HOME=/tmp/shot target/release/laser-check alert examples/duplicated-layer.svg examples/honeycomb-coaster.svg
```

Then capture each window with <kbd>⌘</kbd><kbd>⇧</kbd><kbd>4</kbd>, <kbd>Space</kbd>.
