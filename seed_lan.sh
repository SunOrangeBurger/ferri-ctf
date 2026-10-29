#!/usr/bin/env bash
set -euo pipefail

# Configuration
BASE_URL="${BASE_URL:-http://127.0.0.1:8080}"
BIN_PATH="${BIN_PATH:-./target/release/ferrisctf}"
PASSWORD="Passw0rdTest"
COOKIE_DIR="$(mktemp -d -t ferris_lan_XXXXXX)"
SERVER_STARTED=0

cleanup() {
    rm -rf "$COOKIE_DIR"
    if [ "$SERVER_STARTED" -eq 1 ] && [ -n "${SERVER_PID:-}" ]; then
        echo ""
        echo "Note: Server started by this script is running in the background (PID: $SERVER_PID)."
        echo "To stop it, run: kill $SERVER_PID"
    fi
}
trap cleanup EXIT

extract_csrf() {
    local html="$1"
    local token
    token=$(echo "$html" | grep -o 'name="csrf_token" value="[^"]*"' | head -n 1 | cut -d'"' -f4 || true)
    if [ -z "$token" ]; then
        token=$(echo "$html" | sed -n 's/.*name="csrf_token" value="\([^"]*\)".*/\1/p' | head -n 1 || true)
    fi
    echo "$token"
}

extract_join_code() {
    local html="$1"
    local code
    code=$(echo "$html" | sed -n '/id="join-code"/{n;p;}' | tr -d '[:space:]' || true)
    if [ -z "$code" ]; then
        code=$(echo "$html" | grep -A 2 'id="join-code"' | grep -oE '[0-9A-HJ-NP-Z]{8}' | head -n 1 || true)
    fi
    echo "$code"
}

# -------------------------------------------------------------
# Step 1: Start the server in the background
# -------------------------------------------------------------
echo "============================================================"
echo "Step 1: Starting FerrisCTF Server"
echo "============================================================"

if curl -s -f "$BASE_URL/health" >/dev/null 2>&1; then
    echo "Server is already running and healthy at $BASE_URL."
else
    if [ ! -f "$BIN_PATH" ]; then
        echo "Error: Binary not found at $BIN_PATH."
        echo "Please build the release binary first: cargo build --release"
        exit 1
    fi

    echo "Launching $BIN_PATH in the background..."
    "$BIN_PATH" > server.log 2>&1 &
    SERVER_PID=$!
    SERVER_STARTED=1
    echo "Server process launched with PID $SERVER_PID. Logs written to server.log."

    echo -n "Waiting for server to become healthy at $BASE_URL/health"
    READY=0
    for i in $(seq 1 30); do
        if curl -s -f "$BASE_URL/health" >/dev/null 2>&1; then
            READY=1
            break
        fi
        echo -n "."
        sleep 1
    done
    echo ""

    if [ "$READY" -ne 1 ]; then
        echo "Error: Server failed to start within 30 seconds. Check server.log:"
        tail -n 20 server.log || true
        exit 1
    fi
    echo "Server is online and healthy!"
fi

# -------------------------------------------------------------
# Step 2: Register 125 users (user1 to user125)
# -------------------------------------------------------------
echo ""
echo "============================================================"
echo "Step 2: Registering 125 users (user1 to user125)"
echo "============================================================"

for i in $(seq 1 125); do
    USERNAME="user$i"
    COOKIE_FILE="$COOKIE_DIR/${USERNAME}.cookie"

    # 1. Fetch register page to get CSRF token and ferris_pre cookie
    REG_HTML=$(curl -s -c "$COOKIE_FILE" "$BASE_URL/register")
    CSRF_TOKEN=$(extract_csrf "$REG_HTML")

    if [ -z "$CSRF_TOKEN" ]; then
        echo "Error: Failed to obtain CSRF token for $USERNAME registration"
        exit 1
    fi

    # 2. Submit registration
    HTTP_CODE=$(curl -s -b "$COOKIE_FILE" -c "$COOKIE_FILE" -o /dev/null -w "%{http_code}" \
        -X POST "$BASE_URL/register" \
        --data-urlencode "username=$USERNAME" \
        --data-urlencode "display_name=$USERNAME" \
        --data-urlencode "password=$PASSWORD" \
        --data-urlencode "password_confirm=$PASSWORD" \
        --data-urlencode "csrf_token=$CSRF_TOKEN")

    if [ "$HTTP_CODE" -eq 303 ] || [ "$HTTP_CODE" -eq 200 ]; then
        # Successfully registered and logged in
        printf " [Registered] %-10s (HTTP %s)\n" "$USERNAME" "$HTTP_CODE"
    elif [ "$HTTP_CODE" -eq 409 ]; then
        # Already registered, log in to establish session cookie
        LOGIN_HTML=$(curl -s -c "$COOKIE_FILE" "$BASE_URL/login")
        LOGIN_CSRF=$(extract_csrf "$LOGIN_HTML")
        curl -s -b "$COOKIE_FILE" -c "$COOKIE_FILE" -o /dev/null \
            -X POST "$BASE_URL/login" \
            --data-urlencode "username=$USERNAME" \
            --data-urlencode "password=$PASSWORD" \
            --data-urlencode "csrf_token=$LOGIN_CSRF"
        printf " [Logged in]  %-10s (Already registered)\n" "$USERNAME"
    else
        echo "Warning: Unexpected response HTTP $HTTP_CODE for $USERNAME"
    fi
done

# -------------------------------------------------------------
# Step 3: Create 62 teams using the first 62 users
# -------------------------------------------------------------
echo ""
echo "============================================================"
echo "Step 3: Creating 62 teams (using user1 to user62)"
echo "============================================================"

declare -a JOIN_CODES=()

for i in $(seq 1 62); do
    CAPTAIN="user$i"
    COOKIE_FILE="$COOKIE_DIR/${CAPTAIN}.cookie"
    TEAM_NAME="Team $i"

    # Fetch onboarding page for CSRF token
    ONBOARD_HTML=$(curl -s -b "$COOKIE_FILE" -c "$COOKIE_FILE" "$BASE_URL/onboarding")
    CSRF_TOKEN=$(extract_csrf "$ONBOARD_HTML")

    # If user doesn't have team yet, submit team creation
    if [ -n "$CSRF_TOKEN" ]; then
        curl -s -b "$COOKIE_FILE" -c "$COOKIE_FILE" -o /dev/null \
            -X POST "$BASE_URL/team/create" \
            --data-urlencode "name=$TEAM_NAME" \
            --data-urlencode "csrf_token=$CSRF_TOKEN"
    fi

    # Fetch team dashboard to get join code
    TEAM_HTML=$(curl -s -b "$COOKIE_FILE" -L "$BASE_URL/team")
    JOIN_CODE=$(extract_join_code "$TEAM_HTML")

    # Fallback to database lookup if needed
    if [ -z "$JOIN_CODE" ] && command -v sqlite3 >/dev/null 2>&1; then
        JOIN_CODE=$(sqlite3 ferrisctf.db "SELECT join_code FROM teams WHERE name = '$TEAM_NAME' AND disbanded = 0 LIMIT 1;" 2>/dev/null || true)
    fi

    if [ -z "$JOIN_CODE" ]; then
        echo "Error: Could not retrieve join code for $TEAM_NAME (Captain: $CAPTAIN)"
        exit 1
    fi

    JOIN_CODES[$i]="$JOIN_CODE"
    printf " [Team Created] %-10s | Captain: %-8s | Join Code: %s\n" "$TEAM_NAME" "$CAPTAIN" "$JOIN_CODE"
done

# -------------------------------------------------------------
# Step 4: Join users 63-124 to those teams
# -------------------------------------------------------------
echo ""
echo "============================================================"
echo "Step 4: Joining users 63-124 to teams 1-62"
echo "============================================================"

for i in $(seq 1 62); do
    MEMBER_INDEX=$((62 + i))
    MEMBER="user$MEMBER_INDEX"
    COOKIE_FILE="$COOKIE_DIR/${MEMBER}.cookie"
    JOIN_CODE="${JOIN_CODES[$i]}"
    TEAM_NAME="Team $i"

    # Fetch onboarding page to get CSRF token
    ONBOARD_HTML=$(curl -s -b "$COOKIE_FILE" -c "$COOKIE_FILE" "$BASE_URL/onboarding")
    CSRF_TOKEN=$(extract_csrf "$ONBOARD_HTML")

    if [ -n "$CSRF_TOKEN" ]; then
        HTTP_CODE=$(curl -s -b "$COOKIE_FILE" -c "$COOKIE_FILE" -o /dev/null -w "%{http_code}" \
            -X POST "$BASE_URL/team/join" \
            --data-urlencode "code=$JOIN_CODE" \
            --data-urlencode "csrf_token=$CSRF_TOKEN")
        printf " [Team Joined]  %-8s joined %-10s (Code: %s, HTTP %s)\n" "$MEMBER" "$TEAM_NAME" "$JOIN_CODE" "$HTTP_CODE"
    else
        # User might already be in team
        printf " [Skipped]      %-8s already in team or missing CSRF token\n" "$MEMBER"
    fi
done

echo ""
echo "============================================================"
echo "LAN Seeding Completed Successfully!"
echo "============================================================"
echo "Summary:"
echo "- 125 users registered (user1 to user125) with password: $PASSWORD"
echo "- 62 teams created (Team 1 to Team 62)"
echo "- 62 team captains: user1 to user62"
echo "- 62 team members joined: user63 to user124"
echo "- 1 solo/unassigned participant: user125"
echo "- Server address: $BASE_URL"
echo "============================================================"
