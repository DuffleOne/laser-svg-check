# Releasing

Releases are built by GitHub Actions ([`.github/workflows/release.yml`](../.github/workflows/release.yml)) when a version tag is pushed.

1. Bump `version` in `Cargo.toml`.
2. Add a section for it at the top of [`CHANGELOG.md`](../CHANGELOG.md), headed `## 0.2.0 - YYYY-MM-DD`. That section becomes the release notes.
3. Commit, then tag and push:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

The workflow then:

- runs the tests,
- checks the tag matches `Cargo.toml`,
- builds for Apple silicon and Intel and joins them into one universal binary,
- zips `Laser Check.app` and `Install Laser Check.command` (a copy of `scripts/install-app.sh`) into `Laser-Check-<version>-macOS.zip` with `scripts/package.sh`, plus a SHA-256 checksum,
- publishes a GitHub release with the changelog section and install steps from `.github/release-footer.md`.

If a release build fails, fix it, delete the tag (`git push origin :v0.2.0`) and tag again.

To build the same zip locally (it lands in `dist/`):

```sh
scripts/package.sh
```

## Signing

Builds are ad-hoc signed, not notarised, so on first launch macOS asks people to confirm they want to open it. Notarising needs an Apple Developer ID certificate and an app-specific password stored as repository secrets. Add a `codesign --options runtime --sign "Developer ID Application: …"` step and a `xcrun notarytool submit --wait` step before packaging if that's ever wanted.
