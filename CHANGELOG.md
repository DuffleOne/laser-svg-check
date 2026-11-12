# Changelog

## 0.1.1 - 2026-11-12

- Signed with a Developer ID and notarised by Apple, so macOS opens it without warnings.
- Ships as a disk image: drag the app into Applications and open it. Opening the app starts the watcher at login, so there's no installer script any more. Set `start_at_login = false` to opt out.
- `laser-check uninstall` stops the watcher and removes its login agent.

## 0.1.0 - 2026-11-11

First release.

**Finding doubled lines**

- Checks every path, polygon, polyline, line, rect, circle and ellipse, with transforms applied, for stretches where two lines run along each other within 0.05 mm for at least 2 mm.
- Crossings, touching curves and sharp tips don't count. Neither do invisible shapes (no stroke and no fill), which laser software doesn't cut.
- Groups problems into numbered areas, in reading order, each with its own length.

**Fixing**

- Keeps the first copy of each line, trims the rest, and saves `name-fixed.svg` next to the original. The original is never touched.
- Every fixed copy is checked against its original before it's marked **verified**: no line may go missing and nothing may still be doubled.

**Watching**

- Watches `~/Downloads` (or any folders you set) and alerts when a new SVG would cut twice, with **Fix**, **Review** and **Ignore for 3 days**.
- Remembers files by content hash, so clean files and its own fixed copies are never flagged, and warnings only repeat after `rewarn_days`.
- Uses about 3 MB of memory and no CPU while waiting.

**Reviewing**

- Drop SVGs or folders onto the app. Files are grouped into To fix, Fixed and Clean.
- Numbered markers on the sheet, an inspector listing every area, zoom, and an Original / After fix toggle. <kbd>↑</kbd> <kbd>↓</kbd> step through areas and <kbd>⌘</kbd><kbd>↩</kbd> fixes.

**Also**

- `check`, `fix`, `verify` and `preview` from the command line.
- Universal build for Apple silicon and Intel, macOS 13 or later.
