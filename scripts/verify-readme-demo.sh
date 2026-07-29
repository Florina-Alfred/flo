#!/usr/bin/env bash
# scripts/verify-readme-demo.sh
#
# Manual verification of the README quick-start demo.
# Requires: cargo, a terminal (or tmux/screen), and multicast support.
#
# Usage:
#   chmod +x scripts/verify-readme-demo.sh
#   ./scripts/verify-readme-demo.sh
#
# Or run each step manually in separate terminals.

set -euo pipefail

echo "=== flo README demo verification ==="
echo ""

# ── Step 0: Build ──────────────────────────────────────────────────
echo "[1/5] Building binaries..."
cargo build --bin flo-server --bin flo 2>&1 | tail -1
echo ""

# ── Step 1: Create config files ────────────────────────────────────
echo "[2/5] Creating config files..."

cat > /tmp/flo-server-config.toml << 'TOML'
[[expected_clients]]
robot_id = "robot-7"

[[expected_clients]]
robot_id = "robot-8"
TOML

cat > /tmp/flo-robot-7-config.toml << 'TOML'
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-7/location/x"
y = "robot-7/location/y"
z = "robot-7/location/z"

[default_subscriptions.zone]
site_id = "robot-7/site"
zone_enter = "zone/cell-3/7/enter"
zone_exit = "zone/cell-3/7/exit"

[default_publishers.location]
topic = "robot-7/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-7/zone"
period_ms = 1000
TOML

cat > /tmp/flo-robot-7-rules.toml << 'TOML'
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot-7/local/bumper" },
  { topic = "robot-7/local/imu" },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]
TOML

cat > /tmp/flo-robot-8-config.toml << 'TOML'
[client]
heartbeat_interval_ms = 1000

[default_subscriptions.location]
x = "robot-8/location/x"
y = "robot-8/location/y"
z = "robot-8/location/z"

[default_subscriptions.zone]
site_id = "robot-8/site"
zone_enter = "zone/cell-3/8/enter"
zone_exit = "zone/cell-3/8/exit"

[default_publishers.location]
topic = "robot-8/location"
period_ms = 100

[default_publishers.zone]
topic = "robot-8/zone"
period_ms = 1000
TOML

cat > /tmp/flo-robot-8-rules.toml << 'TOML'
[[rules]]
name = "e-stop-on-bumper"
when.all = [
  { topic = "robot-8/local/bumper" },
  { topic = "robot-8/local/imu" },
]
actions = [
  { topic = "stop/fleet/cmd", qos = "reliable", payload = { stop = true } },
]
TOML

echo "  Config files written to /tmp/flo-*.toml"
echo ""

# ── Step 2: Verify rule check on examples ──────────────────────────
echo "[3/5] Verifying rule check on example files..."
cargo run --bin flo -- rule check examples/rules/hrc-cell.toml
cargo run --bin flo -- rule check examples/rules/warehouse-fleet.toml
echo "  All examples pass."
echo ""

# ── Step 3: Start server ───────────────────────────────────────────
echo "[4/5] Starting flo-server in background..."
cargo run --bin flo-server -- \
  --config /tmp/flo-server-config.toml \
  --auth-mode none \
  --auth-allow-insecure \
  > /tmp/flo-server.log 2>&1 &
SERVER_PID=$!
echo "  Server PID: $SERVER_PID"
echo "  Logs: /tmp/flo-server.log"
sleep 5

# Check server started
if grep -q "flo-engine server mode started" /tmp/flo-server.log; then
  echo "  ✓ Server started successfully"
else
  echo "  ✗ Server failed to start. Log:"
  cat /tmp/flo-server.log
  kill $SERVER_PID 2>/dev/null
  exit 1
fi

# Find the server's listening port
SERVER_PORT=$(ss -tlnp 2>/dev/null | grep "flo-server" | grep -oP '\*:\K[0-9]+' | head -1)
if [ -z "$SERVER_PORT" ]; then
  echo "  ✗ Could not find server port in ss output"
  kill $SERVER_PID 2>/dev/null
  exit 1
fi
echo "  Server port: $SERVER_PORT (tcp/127.0.0.1:$SERVER_PORT)"
echo ""

# ── Step 4: Start clients ──────────────────────────────────────────
echo "[5/5] Starting clients..."

cargo run --bin flo -- \
  --robot-id robot-7 \
  --config /tmp/flo-robot-7-config.toml \
  --ruleset /tmp/flo-robot-7-rules.toml \
  --auth-mode none \
  --auth-allow-insecure \
  --connect "tcp/127.0.0.1:${SERVER_PORT}" \
  > /tmp/flo-robot-7.log 2>&1 &
ROBOT7_PID=$!
echo "  robot-7 PID: $ROBOT7_PID"
echo "  Logs: /tmp/flo-robot-7.log"
sleep 3

cargo run --bin flo -- \
  --robot-id robot-8 \
  --config /tmp/flo-robot-8-config.toml \
  --ruleset /tmp/flo-robot-8-rules.toml \
  --auth-mode none \
  --auth-allow-insecure \
  --connect "tcp/127.0.0.1:${SERVER_PORT}" \
  > /tmp/flo-robot-8.log 2>&1 &
ROBOT8_PID=$!
echo "  robot-8 PID: $ROBOT8_PID"
echo "  Logs: /tmp/flo-robot-8.log"
sleep 5

# ── Verify ─────────────────────────────────────────────────────────
echo ""
echo "=== Verification ==="
echo ""

echo "--- Server log (last 5 lines) ---"
tail -5 /tmp/flo-server.log
echo ""

echo "--- robot-7 log (last 5 lines) ---"
tail -5 /tmp/flo-robot-7.log
echo ""

echo "--- robot-8 log (last 5 lines) ---"
tail -5 /tmp/flo-robot-8.log
echo ""

# Check registration
PASS=true
for robot in robot-7 robot-8; do
  if grep -q "registration successful" /tmp/flo-${robot}.log; then
    echo "✓ ${robot} registered successfully"
  else
    echo "✗ ${robot} registration FAILED"
    PASS=false
  fi
done

# Check health endpoints
echo ""
echo "--- Health endpoints ---"
for robot in robot-7 robot-8; do
  # Find the port from the log (health server listening on 0.0.0.0:8080)
  if curl -sf http://localhost:8080/healthz > /dev/null 2>&1; then
    echo "✓ /healthz → 200 OK"
  else
    echo "✗ /healthz → FAILED"
    PASS=false
  fi

  if curl -sf http://localhost:8080/readyz > /dev/null 2>&1; then
    echo "✓ /readyz → 200 OK"
  else
    echo "✗ /readyz → FAILED (may need more time)"
  fi

  METRICS=$(curl -sf http://localhost:8080/metrics 2>/dev/null || echo "")
  if echo "$METRICS" | grep -q "flo_uptime_seconds"; then
    echo "✓ /metrics → contains flo_uptime_seconds"
  else
    echo "✗ /metrics → FAILED or missing metrics"
  fi
  break  # Only check one client's health server
done

echo ""
echo "=== Cleanup ==="
kill $SERVER_PID $ROBOT7_PID $ROBOT8_PID 2>/dev/null
wait $SERVER_PID $ROBOT7_PID $ROBOT8_PID 2>/dev/null
echo "Processes killed."

echo ""
if [ "$PASS" = true ]; then
  echo "✅ All checks passed!"
else
  echo "❌ Some checks failed — review logs above."
  exit 1
fi
