#!/bin/zsh
# Uploads the Developer ID certificate and a notarisation key to the GitHub
# repo's Actions secrets, so tagged releases come out signed and notarised.
# Run it yourself: it reads straight from your files and keychain export and
# pipes into `gh secret set`, so nothing is printed or saved elsewhere.
set -euo pipefail

REPO=${REPO:-DuffleOne/laser-svg-check}
command -v gh >/dev/null || { echo "needs the GitHub CLI: brew install gh" >&2; exit 1; }

cat <<'EOF'
1/2  Developer ID certificate

In Keychain Access, open the login keychain → My Certificates, right-click
"Developer ID Application: …", choose Export, save it as a .p12 file and give
it a password. (Export the certificate row, so the private key comes with it.)

EOF
read "p12?Path to the .p12 file: "
p12=${~p12}
[[ -f "$p12" ]] || { echo "no file at $p12" >&2; exit 1; }
read -s "p12_pass?Password you gave it: "; echo

base64 -i "$p12" | gh secret set MACOS_CERTIFICATE --repo "$REPO"
printf %s "$p12_pass" | gh secret set MACOS_CERTIFICATE_PASSWORD --repo "$REPO"
unset p12_pass
echo "Certificate saved. You can delete $p12 now."

cat <<'EOF'

2/2  Notarisation (App Store Connect API key)

Uses an App Store Connect API key with the Developer role or higher. Keys and
the issuer ID are in App Store Connect → Users and Access → Integrations →
App Store Connect API.

EOF
keys=(~/.appstoreconnect/private_keys/AuthKey_*.p8(N) ~/private_keys/AuthKey_*.p8(N))
if (( ${#keys} == 1 )); then
  key=${keys[1]}
  echo "Found ${key:t}"
else
  read "key?Path to the AuthKey_XXXXXXXXXX.p8 file: "
  key=${~key}
fi
[[ -f "$key" ]] || { echo "no file at $key" >&2; exit 1; }
key_id=${${key:t:r}#AuthKey_}
read "issuer?Issuer ID (a UUID): "

base64 -i "$key" | gh secret set NOTARY_KEY --repo "$REPO"
printf %s "$key_id" | gh secret set NOTARY_KEY_ID --repo "$REPO"
printf %s "$issuer" | gh secret set NOTARY_ISSUER --repo "$REPO"

echo
gh secret list --repo "$REPO"
echo
echo "Done. The next v* tag builds a signed, notarised release."
