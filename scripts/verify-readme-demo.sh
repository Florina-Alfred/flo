#!/usr/bin/env bash
# scripts/verify-readme-demo.sh — trimmed smoke test for README Quickstart.
# Verifies the loopback demo with explicit --connect (no multicast, no ss/lsof).
set -euo pipefail

echo "=== flo README demo verification ==="

# Build
echo "[1/4] Building binaries..."
cargo build --bin flo-server --bin flo 2>&1 | tail -1

# Configs
echo "[2/4] Creating config files..."
cat > /tmp/flo-server-config.toml <<'TOML'
[[expected_clients]]
robot_id = "robot-7"
[[expected_clients]]
robot_id = "robot-8"
TOML

# Use the canonical fixture as the template (now included in Cargo package)
cp tests/fixtures/minimal-client-config.toml /tmp/flo-robot-7-config.toml
cp tests/fixtures/minimal-client-config.toml /tmp/flo-robot-8-config.toml
sed -i 's/robot-7/robot-8/g' /tmp/flo-robot-8-config.toml 2>/dev/null || true

cat > /tmp/flo-robot-7-rules.toml <<'TOML'
[[rules]]
name = "e-stop-on-bumper"
when.all = [{ topic = "robot-7/local/bumper" }, { topic = "robot-7/local/imu" }]
actions = [{ topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } }]
TOML
cat > /tmp/flo-robot-8-rules.toml <<'TOML'
[[rules]]
name = "e-stop-on-bumper"
when.all = [{ topic = "robot-8/local/bumper" }, { topic = "robot-8/local/imu" }]
actions = [{ topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } }]
TOML

# Rule check
echo "[3/4] Verifying rule check..."
cargo run --bin flo -- rule check examples/rules/hrc-cell.toml
cargo run --bin flo -- rule check examples/rules/warehouse-fleet.toml
cargo run --bin flo -- --print-zenoh-port --auth-mode none --auth-allow-insecure 2>&1 | grep -q "tcp/" && echo "  --print-zenoh-port works"

# Server
echo "[4/4] Starting flo-server..."
cargo run --bin flo-server -- --config /tmp/flo-server-config.toml --auth-mode none --auth-allow-insecure > /tmp/flo-server.log 2>&1 &
SERVER_PID=$!
sleep 5
grep -q "flo-engine server mode started" /tmp/flo-server.log || { cat /tmp/flo-server.log; kill $SERVER_PID 2>/dev/null; exit 1; }
echo "  ✓ Server started (PID $SERVER_PID)"

# Discover Zenoh port via log (no ss/lsof fallback — use transport.locators())
ZENOH_PORT=$(grep -oE 'zenoh router listening.*tcp/127\.0\.0\.1:[0-9]+' /tmp/flo-server.log | grep -oE '[0-9]+$' | head -1 || true)
if [ -z "$ZENOH_PORT" ]; then echo "  ✗ Could not find Zenoh port in log"; cat /tmp/flo-server.log; kill $SERVER_PID 2>/dev/null; exit 1; fi
echo "  Zenoh port: $ZENOH_PORT (from log, distinct from health port)"

# Clients
for id in 7 8; do
  cargo run --bin flo -- --robot-id robot-$id --config /tmp/flo-robot-$id-config.toml --ruleset /tmp/flo-robot-$id-rules.toml --auth-mode none --auth-allow-insecure --connect "tcp/127.0.0.1:${ZENOH_PORT}" > /tmp/flo-robot-$id.log 2>&1 &
  echo "  robot-$id PID $!"
done
sleep 5

# Verify
echo "=== Verification ==="
PASS=true
for robot in robot-7 robot-8; do
  if grep -q "registration successful" /tmp/flo-${robot}.log; then echo "✓ ${robot} registered"; else echo "✗ ${robot} FAILED"; PASS=false; fi
done
HEALTH_PORT=$(grep 'health server listening' /tmp/flo-robot-7.log 2>/dev/null | grep -oE '0\.0\.0\.0:[0-9]+' | grep -oE '[0-9]+$' | head -1 || true)
if [ -n "$HEALTH_PORT" ]; then
  curl -sf --connect-timeout 2 --max-time 5 "http://127.0.0.1:${HEALTH_PORT}/healthz" > /dev/null && echo "✓ /healthz 200" || { echo "✗ /healthz FAILED"; PASS=false; }
  curl -sf --connect-timeout 2 --max-time 5 "http://127.0.0.1:${HEALTH_PORT}/metrics" 2>/dev/null | grep -q "flo_uptime_seconds" && echo "✓ /metrics ok" || echo "✗ /metrics FAILED"
fi

kill $SERVER_PID $(jobs -p) 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true
if [ "$PASS" = true ]; then echo "✅ All checks passed!"; else echo "❌ Some checks failed"; exit 1; fi
