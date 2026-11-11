# Security

Laser Check runs only on your Mac. It reads SVGs from the folders you point it at, writes `-fixed.svg` copies next to them, and keeps a small history file in `~/Library/Application Support/laser-check`. It makes no network requests.

If you find a way a crafted SVG could make it do something it shouldn't (read or write outside those places, run code, crash the watcher in a loop), please report it privately through [GitHub's security advisory form](https://github.com/DuffleOne/laser-svg-check/security/advisories/new) rather than a public issue.

Only the latest release gets fixes.
