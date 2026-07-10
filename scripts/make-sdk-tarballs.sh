#!/usr/bin/env bash
# Build the two SDK artifacts HUSH fetches from the CDN:
#   afx-runtime-<arch>.tar.zst — full runtime (nvafx + denoiser feature + CUDA/TensorRT stack)
#   afx-link-<arch>.tar.zst    — just libnv_audiofx + libcudart, for link-time (CI/AUR/Nix)
# Both preserve the SDK-relative layout + symlinks. Prints sha256 + size for the
# manifests in hush-core/src/sdk.rs. Upload the outputs to R2 (sdk/<ver>/) manually.
set -euo pipefail

SDK="${NVAFX_SDK:-$HOME/maxine-dl/sdk/Audio_Effects_SDK}"
VER="${1:-2.1.0}"
ARCH="$(uname -m)"
OUT="${OUT_DIR:-$(dirname "$0")/../dist/sdk}"
mkdir -p "$OUT"

[ -f "$SDK/nvafx/lib/libnv_audiofx.so.2.1.0" ] || { echo "SDK not found at $SDK"; exit 1; }

runtime="$OUT/afx-runtime-${ARCH}.tar.zst"
link="$OUT/afx-link-${ARCH}.tar.zst"

echo "== building runtime tarball (this takes a while — ~2.3 GB → ~1.2 GB) =="
tar -C "$SDK" -cf - \
  nvafx/lib \
  features/denoiser/lib \
  external/cuda/lib \
  licenses \
  | zstd -19 --long=27 -T0 -o "$runtime" -f

echo "== building link tarball =="
# Flat layout: just the two libs (+ their symlink chains) the linker/loader need.
tmp="$(mktemp -d)"
mkdir -p "$tmp/nvafx/lib" "$tmp/external/cuda/lib"
cp -a "$SDK"/nvafx/lib/libnv_audiofx.so* "$tmp/nvafx/lib/"
cp -a "$SDK"/external/cuda/lib/libcudart.so* "$tmp/external/cuda/lib/"
tar -C "$tmp" -cf - . | zstd -19 -T0 -o "$link" -f
rm -rf "$tmp"

echo
echo "== manifest entries (fill into hush-core/src/sdk.rs) =="
for f in "$runtime" "$link"; do
  printf '%-40s size=%-12s sha256=%s\n' "$(basename "$f")" "$(stat -c%s "$f")" "$(sha256sum "$f" | cut -d' ' -f1)"
done
echo
echo "Upload to:  s3://hush/sdk/${VER}/afx-{runtime,link}-${ARCH}.tar.zst"
