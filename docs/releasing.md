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
- signs and notarises the app, puts it in `Laser-Check-<version>.dmg` beside a link to Applications, then signs and notarises the image too (`scripts/package.sh`), plus a SHA-256 checksum,
- publishes a GitHub release with the changelog section and install steps from `.github/release-footer.md`.

If a release build fails, fix it, delete the tag (`git push origin :v0.2.0`) and tag again.

To build the same image locally (it lands in `dist/`):

```sh
scripts/package.sh
```

## Signing and notarising

Releases are signed with a Developer ID certificate and notarised by Apple when these repository secrets are set:

| Secret | What |
| --- | --- |
| `MACOS_CERTIFICATE` | The "Developer ID Application" certificate and its private key, exported as .p12, base64 |
| `MACOS_CERTIFICATE_PASSWORD` | The password the .p12 was exported with |
| `NOTARY_KEY` | An App Store Connect API key (.p8), base64. Developer role or higher |
| `NOTARY_KEY_ID` | That key's ID |
| `NOTARY_ISSUER` | The issuer ID from App Store Connect → Users and Access → Integrations |

`scripts/set-signing-secrets.sh` walks through exporting the certificate and uploads all five with `gh secret set`, without printing any of them.

With the secrets in place, the workflow imports the certificate into a throwaway keychain, `bundle.sh` signs with the hardened runtime, `package.sh` sends the app to Apple's notary service and staples the ticket, and the build checks Gatekeeper accepts it before publishing. The keychain and key are deleted at the end of the job.

Without them, the release is signed ad hoc and macOS won't open it without a trip to Privacy & Security.

There's no installer script in the image because Apple only notarises signed code, and a shell script can't carry a ticket. Opening the app sets up the login agent instead (`src/agent.rs`).

To sign a local build the same way: `SIGN_IDENTITY="Developer ID Application" scripts/package.sh`. Add `NOTARY_KEY_PATH`, `NOTARY_KEY_ID` and `NOTARY_ISSUER` to notarise it too.
