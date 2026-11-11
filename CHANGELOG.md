# Changelog

## 0.1.0 - 2026-11-11

First release.

- Watches `~/Downloads` (or any folders you set) for new SVGs and alerts when lines would be cut more than once.
- Alert with **Fix it**, **Show me** and **Fix and show me**.
- Review window: drop in SVGs or folders, see doubled lines in red, step through problem areas, fix one file or all of them.
- Fixes keep the first copy of each line, trim the rest, and save `name-fixed.svg` next to the original.
- Remembers files by content hash: clean files are skipped, warnings repeat after 3 days.
- `check`, `fix`, `verify` and `preview` from the command line.
- Universal build for Apple silicon and Intel, macOS 13 or later.
