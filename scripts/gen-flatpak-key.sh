#!/usr/bin/env bash
# One-time: generate the GPG key that signs the self-hosted Flatpak repo.
# Prints the private key (for the FLATPAK_GPG_PRIVATE_KEY GitHub secret) and the
# fingerprint (for the FLATPAK_GPG_FINGERPRINT repo variable), and writes the
# public key + .flatpakrepo users install from.
set -euo pipefail
cd "$(dirname "$0")/.."

FPR=$(gpg --batch --quick-gen-key "HUSH Flatpak Repo <support@stockimg.ai>" ed25519 sign never 2>&1 \
  | grep -oE '[A-F0-9]{40}' | head -1)
[ -n "$FPR" ] || { echo "key generation failed"; exit 1; }

gpg --export "$FPR" > packaging/flatpak/hush.gpg
B64=$(base64 -w0 packaging/flatpak/hush.gpg)
cat > packaging/flatpak/hush.flatpakrepo <<EOF
[Flatpak Repo]
Title=HUSH
Url=https://umceko.github.io/hush/
Homepage=https://github.com/UMCEKO/hush
Description=NVIDIA Maxine denoiser virtual microphone
GPGKey=$B64
EOF

echo "== fingerprint (set repo variable FLATPAK_GPG_FINGERPRINT) =="
echo "$FPR"
echo
echo "== private key (set repo secret FLATPAK_GPG_PRIVATE_KEY to the block below) =="
gpg --export-secret-keys --armor "$FPR"
echo
echo "Wrote packaging/flatpak/hush.gpg + hush.flatpakrepo — commit both."
