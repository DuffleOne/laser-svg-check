# Contributing

Thanks for wanting to help. Laser Check is small and meant to stay that way: one job (no line cut twice), done carefully, on a Mac.

## Good first contributions

- **SVGs that trip it up.** A file it flags wrongly, misses a doubled line in, or fixes badly is the most useful thing you can send. Open a [bug report](https://github.com/DuffleOne/laser-svg-check/issues/new?template=bug_report.yml) and attach it, cut down to just the problem if you can.
- **Exports from other tools.** Inkscape, Affinity, CorelDRAW and Fusion all write SVG slightly differently. Examples help.
- **Docs.** If something in the README confused you, it'll confuse the next person too.

## Working on the code

Everything you need is in [docs/development.md](docs/development.md): setup, the project layout, running the watcher without touching your real settings, and some gpui gotchas.

```sh
cargo test --release
cargo clippy --release --all-targets -- -D warnings
cargo fmt --check
```

CI runs the same three, plus a fix-and-verify pass over `examples/`.

If you change how doubled lines are found or fixed, run it over some real exports and check nothing went missing:

```sh
laser-check fix some/folder
for f in some/folder/*-fixed.svg; do laser-check verify "${f%-fixed.svg}.svg" "$f"; done
```

The rule the fix must never break: **every line that would have been cut is still cut.** A fix that removes a little too little is a nuisance; one that removes too much ruins a sheet of material.

## Pull requests

- Keep each one to a single change, and say why in the description.
- Add a test in `src/main.rs` when you fix a detection or fixing bug. Small hand-written SVGs are fine.
- UI changes need a screenshot. Dark mode is the designed theme, and its colours are the tokens in `src/ui/theme.rs`. Use those rather than new ones.
- Write the way the rest of the docs are written: plain words, British spelling.

## Releases

See [docs/releasing.md](docs/releasing.md). In short: bump the version, add a changelog entry, push a `v*` tag.

## Behaviour

Be kind. This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
