#!/usr/bin/env bash
# Read over a real XRootD server with TLS and token (`ztn`) authentication —
# runs on this machine, not CI (it needs an `xrootd` binary, 5.x or later).
#
#   bash scripts/xrootd_tls_local.sh
#
# It starts `xrootd` on a free local port with the test certificate in
# fixtures/xrootd_tls, TLS required for everything (`xrootd.tls all`) and
# `ztn` as the only authentication (`-tokenlib none`: any token is accepted, so
# this tests the transport, not token validation). Then it runs the ignored
# `live_xrootd_with_tls_and_a_token` test with a token in BEARER_TOKEN, once
# with `roots://` and once with `root://` (the server switches it to TLS).

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
XROOTD="${XROOTD:-$(command -v xrootd || true)}"
if [ -z "$XROOTD" ]; then
  echo "SKIP: no xrootd binary (set XROOTD=/path/to/xrootd)"; exit 0
fi

WORK="$(mktemp -d)"
cleanup() {
  [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$WORK/data" "$WORK/run"
cp "$REPO/fixtures/th1d_zstd.root" "$WORK/data/th1d.root"
# xrootd refuses a private key that others can read; git does not keep modes.
cp "$REPO/fixtures/xrootd_tls/server.key" "$WORK/server.key"
chmod 600 "$WORK/server.key"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"

cat > "$WORK/xrd.cfg" <<CFG
xrd.port $PORT
all.export /
oss.localroot $WORK/data
xrd.tls $REPO/fixtures/xrootd_tls/server.pem $WORK/server.key
xrd.tlsca noverify
xrootd.tls all
sec.protocol ztn -tokenlib none
sec.protbind * only ztn
xrootd.seclib libXrdSec.so
CFG

"$XROOTD" -c "$WORK/xrd.cfg" -l "$WORK/run/xrd.log" -n oxiroot-tls >"$WORK/run/stdout" 2>&1 &
PID=$!
LOG="$WORK/run/oxiroot-tls/xrd.log"
for _ in $(seq 1 50); do
  grep -q "initialization completed" "$LOG" 2>/dev/null && break
  if grep -q "initialization failed" "$LOG" 2>/dev/null; then cat "$LOG"; exit 1; fi
  sleep 0.2
done
grep -q "initialization completed" "$LOG" || { echo "xrootd did not start"; cat "$LOG"; exit 1; }
echo "xrootd with TLS + ztn on port $PORT"

cd "$REPO"
for scheme in roots root; do
  echo "──── $scheme:// ────"
  BEARER_TOKEN="eyJhbGciOiJub25lIn0.eyJzdWIiOiJveGlyb290In0.c2ln" \
  OXIROOT_ROOTS_URL="$scheme://localhost:$PORT//th1d.root" \
    cargo test -q -p oxiroot --features xrootd-tls --test it \
      remote_xrootd::tls::live_xrootd_with_tls_and_a_token -- --ignored
done
echo "PASS: TLS + token reads over a real xrootd"
