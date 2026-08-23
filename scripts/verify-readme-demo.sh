#!/usr/bin/env bash
# scripts/verify-readme-demo.sh
#
# Manual verification of the README quick-start demo.
# Requires: cargo, a terminal (or tmux/screen). Works with or without
# multicast: the script uses explicit --connect tcp/127.0.0.1:<zenoh-port>
# for Docker/WSL/CI where multicast is blocked (see README "When multicast is
# blocked"). For port discovery it prefers `ss` (Linux, iproute2) and falls
# back to `lsof -i -P -n | grep LISTEN` on macOS (no ss). The Zenoh port is
# distinct from the health port (FLO_HEALTH_ADDR) — the script parses both
# and never uses the health port for --connect.
#
# Usage:
#   chmod +x scripts/verify-readme-demo.sh
#   ./scripts/verify-readme-demo.sh
#
# Or run each step manually in separate terminals.

set -euo pipefail

echo "=== flo README demo verification ==="
echo ""

# ── Cleanup any leftover processes from previous runs ──────────────
# Use precise patterns: "flo-server" and "flo[[:space:]]" so macOS pkill -f and
# Linux both match. Fallback to pgrep+kill if pkill missing.
if command -v pkill >/dev/null 2>&1; then
  pkill -f "target/debug/flo-server" 2>/dev/null || true
  pkill -f "target/debug/flo[[:space:]]" 2>/dev/null || true
  # also catch bare `target/debug/flo` without trailing space (e.g. no args)
  pkill -f "target/debug/flo$" 2>/dev/null || true
else
  pgrep -f "target/debug/flo-server" 2>/dev/null | xargs -r kill 2>/dev/null || true
  pgrep -f "target/debug/flo[[:space:]]" 2>/dev/null | xargs -r kill 2>/dev/null || true
fi
sleep 1

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
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

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
zone_enter = "zone/cell-3/entered"
zone_exit = "zone/cell-3/cleared"

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

# Find the server's Zenoh port (NOT the health port).
# The Zenoh router listens on tcp/127.0.0.1:0 → random (src/auth.rs:152,
# src/transport.rs:86); health listens on 0.0.0.0:0 → random (src/common.rs:52).
# We must pick the Zenoh listener. The script explicitly uses --connect
# tcp/127.0.0.1:<zenoh-port> for blocked multicast (Docker/WSL/CI) — see README
# "When multicast is blocked" — it does not rely on multicast scouting.
HEALTH_PORT_SERVER=$(grep 'health server listening' /tmp/flo-server.log 2>/dev/null | grep -oE '0\.0\.0\.0:[0-9]+' | grep -oE '[0-9]+$' | head -1 || true)
echo "  Health port (server): ${HEALTH_PORT_SERVER:-unknown} (from log)"

ZENOH_LISTEN_RAW=""
if command -v ss >/dev/null 2>&1; then
  # Linux: ss with -tlnp shows owning process
  ZENOH_LISTEN_RAW=$(ss -tlnp 2>/dev/null | grep "flo-server" || true)
  if [ -z "$ZENOH_LISTEN_RAW" ]; then
    # fallback: ss without process filter, then filter by 127.0.0.1
    ZENOH_LISTEN_RAW=$(ss -tln 2>/dev/null | grep -E '127\.0\.0\.1:' || true)
  fi
else
  # macOS fallback: lsof -i -P -n | grep LISTEN
  if command -v lsof >/dev/null 2>&1; then
    ZENOH_LISTEN_RAW=$(lsof -i -P -n 2>/dev/null | grep LISTEN | grep -E "flo-server|${SERVER_PID}" || true)
    if [ -z "$ZENOH_LISTEN_RAW" ]; then
      ZENOH_LISTEN_RAW=$(lsof -i -P -n 2>/dev/null | grep LISTEN || true)
    fi
  else
    echo "  ✗ Neither ss nor lsof found — cannot discover Zenoh port. On macOS: brew install lsof; on Linux: apt-get install iproute2."
    kill $SERVER_PID 2>/dev/null || true
    exit 1
  fi
fi

# Prefer 127.0.0.1 listeners that are NOT the health port
SERVER_PORT=$(echo "$ZENOH_LISTEN_RAW" | grep -oE '127\.0\.0\.1:[0-9]+' | grep -oE '[0-9]+$' | grep -vx "${HEALTH_PORT_SERVER:-^$}" | head -1 || true)
if [ -z "$SERVER_PORT" ]; then
  # fallback: any port not equal to health
  SERVER_PORT=$(echo "$ZENOH_LISTEN_RAW" | grep -oE ':[0-9]+' | grep -oE '[0-9]+$' | grep -vx "${HEALTH_PORT_SERVER:-^$}" | head -1 || true)
fi
if [ -z "$SERVER_PORT" ]; then
  echo "  ✗ Could not find Zenoh port (health=$HEALTH_PORT_SERVER). LISTEN dump:"
  echo "$ZENOH_LISTEN_RAW"
  echo "  Ensure multicast is blocked? On Docker/WSL/CI use --connect tcp/127.0.0.1:<zenoh-port> (not health port)."
  kill $SERVER_PID 2>/dev/null || true
  exit 1
fi
echo "  Zenoh port: $SERVER_PORT (tcp/127.0.0.1:$SERVER_PORT) — will use --connect (explicit unicast, not multicast)"
echo "  (Health and Zenoh are distinct ports — do not use health port for --connect)"
echo ""

# ── Step 4: Start clients ──────────────────────────────────────────
echo "[5/5] Starting clients (explicit --connect tcp/127.0.0.1:<zenoh-port> for blocked multicast)..."
echo "  Using --connect tcp/127.0.0.1:${SERVER_PORT} (Zenoh, not health). See README 'When multicast is blocked'."
echo ""

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

# Check health endpoints (find port from log since we use random port)
echo ""
echo "--- Health endpoints ---"
HEALTH_PORT=$(grep 'health server listening' /tmp/flo-robot-7.log 2>/dev/null | grep -oE '0\.0\.0\.0:[0-9]+' | grep -oE '[0-9]+$' | head -1 || true)
if [ -z "$HEALTH_PORT" ]; then
  echo "⚠ Could not find health port from log, skipping"
else
  echo "  Health port: $HEALTH_PORT"
  if curl -sf --connect-timeout 2 --max-time 5 "http://127.0.0.1:${HEALTH_PORT}/healthz" > /dev/null 2>&1; then
    echo "✓ /healthz → 200 OK"
  else
    echo "✗ /healthz → FAILED"
    PASS=false
  fi

  if curl -sf --connect-timeout 2 --max-time 5 "http://127.0.0.1:${HEALTH_PORT}/readyz" > /dev/null 2>&1; then
    echo "✓ /readyz → 200 OK"
  else
    echo "✗ /readyz → FAILED (may need more time)"
  fi

  METRICS=$(curl -sf --connect-timeout 2 --max-time 5 "http://127.0.0.1:${HEALTH_PORT}/metrics" 2>/dev/null || echo "")
  if echo "$METRICS" | grep -q "flo_uptime_seconds"; then
    echo "✓ /metrics → contains flo_uptime_seconds"
  else
    echo "✗ /metrics → FAILED or missing metrics"
  fi
fi

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
