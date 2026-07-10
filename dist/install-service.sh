#!/usr/bin/env bash
# Install the HUSH denoiser daemon as a systemd *user* service.
#   ./dist/install-service.sh           # build, install, enable + start now
#   ./dist/install-service.sh --no-start # install only
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
bindir="$HOME/.local/bin"
unitdir="$HOME/.config/systemd/user"

echo "==> building hushd"
( cd "$here" && cargo build --release --bin hushd )

echo "==> installing binary -> $bindir/hushd"
mkdir -p "$bindir"
install -m755 "$here/target/release/hushd" "$bindir/hushd"

echo "==> installing unit -> $unitdir/hush.service"
mkdir -p "$unitdir"
install -m644 "$here/dist/hush.service" "$unitdir/hush.service"

systemctl --user daemon-reload

if [[ "${1:-}" != "--no-start" ]]; then
  echo "==> enabling + starting hush.service"
  systemctl --user enable --now hush.service
  sleep 1
  systemctl --user --no-pager status hush.service | head -6 || true
fi

echo
echo "Done. The GUI ('hush') will attach to this service; closing it leaves audio running."
echo "Logs:   journalctl --user -u hush.service -f"
echo "Stop:   systemctl --user stop hush.service"
