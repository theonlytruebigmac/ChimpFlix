#!/usr/bin/env bash
# ChimpFlix demo setup script.
# Starts the isolated demo stack, creates the owner account via the API,
# applies seed data, and opens the browser to http://localhost:3001.
#
# Usage:
#   bash scripts/demo/setup.sh [--username demo --password chimpflix2026]
#
# Prerequisites: Docker (with Compose v2), sqlite3, curl.
# Does NOT affect your production containers or database.

set -euo pipefail

COMPOSE_FILE="docker-compose.demo.yml"
DEMO_DATA_DIR="./data-demo"
SERVER_URL="http://localhost:8081"
WEB_URL="http://localhost:3001"
DB_PATH="${DEMO_DATA_DIR}/chimpflix.db"

DEMO_USERNAME="${DEMO_USERNAME:-demo}"
DEMO_PASSWORD="${DEMO_PASSWORD:-chimpflix2026}"
DEMO_EMAIL="${DEMO_EMAIL:-demo@localhost}"

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --username) DEMO_USERNAME="$2"; shift 2 ;;
        --password) DEMO_PASSWORD="$2"; shift 2 ;;
        --email)    DEMO_EMAIL="$2";    shift 2 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

echo ""
echo "╔══════════════════════════════════════════╗"
echo "║     ChimpFlix Demo Setup                 ║"
echo "╚══════════════════════════════════════════╝"
echo ""

# ── Step 1: Build and start the server only ───────────────────────────────────
echo "▶ Building and starting demo server (port 8081)..."
mkdir -p "${DEMO_DATA_DIR}"

docker compose -p chimpflix-demo -f "${COMPOSE_FILE}" up -d --build server

# ── Step 2: Wait for the server to be healthy ─────────────────────────────────
echo "▶ Waiting for server to become healthy..."
MAX_WAIT=90
ELAPSED=0
until curl -fsS "${SERVER_URL}/health" > /dev/null 2>&1; do
    sleep 2
    ELAPSED=$((ELAPSED + 2))
    if [[ $ELAPSED -ge $MAX_WAIT ]]; then
        echo "✗ Server did not become healthy within ${MAX_WAIT}s." >&2
        echo "  Check logs: docker compose -f ${COMPOSE_FILE} logs server" >&2
        exit 1
    fi
    echo "  ... still waiting (${ELAPSED}s / ${MAX_WAIT}s)"
done
echo "✓ Server is healthy."

# ── Step 3: Check if setup is already done ────────────────────────────────────
SETUP_NEEDED=$(curl -fsS "${SERVER_URL}/api/v1/auth/status" | grep -o '"setup_needed":true' || true)
if [[ -z "$SETUP_NEEDED" ]]; then
    echo "✓ Setup already completed (owner account exists). Skipping account creation."
else
    echo "▶ Creating owner account (username: ${DEMO_USERNAME})..."
    HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
        -X POST "${SERVER_URL}/api/v1/auth/setup" \
        -H "Content-Type: application/json" \
        -H "Origin: http://localhost:8081" \
        -d "{\"username\":\"${DEMO_USERNAME}\",\"password\":\"${DEMO_PASSWORD}\",\"display_name\":\"Demo User\",\"email\":\"${DEMO_EMAIL}\"}")

    if [[ "$HTTP_STATUS" -ge 200 && "$HTTP_STATUS" -lt 300 ]]; then
        echo "✓ Owner account created: ${DEMO_USERNAME} / ${DEMO_PASSWORD}"
    else
        echo "✗ Setup API returned HTTP ${HTTP_STATUS}." >&2
        echo "  The server may require CHIMPFLIX_SETUP_TOKEN." >&2
        echo "  Complete setup manually at ${WEB_URL} and re-run this script." >&2
        exit 1
    fi
fi

# ── Step 4: Apply seed data ───────────────────────────────────────────────────
echo "▶ Stopping server to apply seed data..."
docker compose -p chimpflix-demo -f "${COMPOSE_FILE}" stop server

echo "▶ Applying demo seed data to ${DB_PATH}..."
if ! command -v sqlite3 &> /dev/null; then
    echo "✗ sqlite3 not found. Install it (e.g. sudo apt install sqlite3) and re-run." >&2
    docker compose -p chimpflix-demo -f "${COMPOSE_FILE}" start server
    exit 1
fi

sqlite3 "${DB_PATH}" < scripts/demo/seed.sql
echo "✓ Seed data applied."

# ── Step 5: Start all services ────────────────────────────────────────────────
echo "▶ Starting all demo services..."
docker compose -p chimpflix-demo -f "${COMPOSE_FILE}" up -d

echo ""
echo "╔══════════════════════════════════════════╗"
echo "║  Demo ready!                             ║"
echo "║                                          ║"
echo "║  URL:      http://localhost:3001         ║"
echo "║  Username: ${DEMO_USERNAME}$(printf '%*s' $((20 - ${#DEMO_USERNAME})) '')║"
echo "║  Password: ${DEMO_PASSWORD}$(printf '%*s' $((20 - ${#DEMO_PASSWORD})) '')║"
echo "╚══════════════════════════════════════════╝"
echo ""
echo "To stop the demo:  docker compose -f ${COMPOSE_FILE} down"
echo "To reset and start fresh:"
echo "  docker compose -f ${COMPOSE_FILE} down"
echo "  rm -rf ${DEMO_DATA_DIR}"
echo "  bash scripts/demo/setup.sh"
echo ""

# Open browser if available
if command -v xdg-open &> /dev/null; then
    xdg-open "${WEB_URL}" &
elif command -v open &> /dev/null; then
    open "${WEB_URL}" &
fi
