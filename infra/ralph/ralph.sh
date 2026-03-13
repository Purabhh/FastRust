#!/bin/bash
# Ralph — FastRust Automated Quality Daemon
# Runs cargo check/test/clippy + custom checks, dispatches issues via hybrid system
# Layers: git task files (persistent) + webhooks (instant) + Discord (human visibility)

set -u

source /opt/ralph/ralph.conf

# Source dispatch functions (hybrid task system)
DISPATCH_CONF="/opt/ralph/dispatch.conf"
if [ -f /opt/ralph/dispatch.sh ]; then
    source /opt/ralph/dispatch.sh
    DISPATCH_AVAILABLE=true
else
    DISPATCH_AVAILABLE=false
fi

TASKS_DIR="${REPO_DIR}/.tasks"

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="${REPORT_DIR}/report_${TIMESTAMP}.json"

# --- Helper: send Discord message ---
discord_send() {
    local token="$1"
    local message="$2"
    curl -s -X POST "https://discord.com/api/v10/channels/${DISCORD_CHANNEL}/messages" \
        -H "Authorization: Bot ${token}" \
        -H "Content-Type: application/json" \
        -d "{\"content\": $(printf '%s' "$message" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))')}" \
        > /dev/null 2>&1
}

# --- Helper: assign file to bot token (cross-dispatch: send AS the other bot) ---
get_dispatch_token() {
    local file="$1"
    case "$file" in
        *app.rs|*router/*|*middleware*|*state.rs|*ws.rs|*sse.rs|*static_files*|*examples/*)
            # PeensBot owns these — dispatch as Penemaster so PeensBot sees it
            echo "$PENEMASTER_TOKEN"
            ;;
        *extract*|*response.rs|*sanitize*|*cookie*|*multipart*|*request.rs|*error.rs)
            # Penemaster owns these — dispatch as PeensBot so Penemaster sees it
            echo "$PEENSBOT_TOKEN"
            ;;
        *)
            # Claude-owned files — notify both, use PeensBot token
            echo "$PEENSBOT_TOKEN"
            ;;
    esac
}

get_owner() {
    local file="$1"
    case "$file" in
        *app.rs|*router/*|*middleware*|*state.rs|*ws.rs|*sse.rs|*static_files*|*examples/*)
            echo "PeensBot"
            ;;
        *extract*|*response.rs|*sanitize*|*cookie*|*multipart*|*request.rs|*error.rs)
            echo "Penemaster"
            ;;
        *)
            echo "Claude"
            ;;
    esac
}

# --- Setup: clone or pull ---
if [ ! -d "$REPO_DIR" ]; then
    git clone "$REPO_URL" "$REPO_DIR"
fi

cd "$REPO_DIR"
git fetch origin main 2>/dev/null
git reset --hard origin/main 2>/dev/null

COMMIT=$(git rev-parse --short HEAD)
COMMIT_FULL=$(git rev-parse HEAD)

# --- Source Rust environment ---
if [ -f /home/openclaw/.cargo/env ]; then
    source /home/openclaw/.cargo/env
elif [ -f /root/.cargo/env ]; then
    source /root/.cargo/env
fi

# --- Run checks ---
echo "Ralph: checking commit ${COMMIT}..."

# Cargo check
CHECK_OUTPUT=$(cargo check --workspace 2>&1) && CHECK_OK=true || CHECK_OK=false
CHECK_ERRORS=""
if [ "$CHECK_OK" = false ]; then
    CHECK_ERRORS=$(echo "$CHECK_OUTPUT" | grep "^error" | head -20)
fi

# Cargo test (only if check passed — no point testing broken code)
TEST_PASSED=0
TEST_FAILED=0
TEST_TOTAL=0
TEST_ERRORS=""
if [ "$CHECK_OK" = true ]; then
    TEST_OUTPUT=$(cargo test --workspace 2>&1) && TEST_OK=true || TEST_OK=false
    # Sum all "N passed" from test result lines
    TEST_PASSED=$(echo "$TEST_OUTPUT" | grep -oP '\d+ passed' | grep -oP '\d+' | awk '{s+=$1}END{print s+0}')
    TEST_FAILED=$(echo "$TEST_OUTPUT" | grep -oP '\d+ failed' | grep -oP '\d+' | awk '{s+=$1}END{print s+0}')
    TEST_TOTAL=$((TEST_PASSED + TEST_FAILED))
    if [ "$TEST_OK" = false ]; then
        TEST_ERRORS=$(echo "$TEST_OUTPUT" | grep -A2 "FAILED\|panicked" | head -20)
    fi
else
    TEST_OK=false
    TEST_ERRORS="skipped (cargo check failed)"
fi

# Cargo clippy (only if check passed)
CLIPPY_WARNINGS=0
CLIPPY_ERRORS=""
if [ "$CHECK_OK" = true ]; then
    CLIPPY_OUTPUT=$(cargo clippy --workspace 2>&1) && CLIPPY_OK=true || CLIPPY_OK=false
    CLIPPY_WARNINGS=$(echo "$CLIPPY_OUTPUT" | grep -c "^warning\[" 2>/dev/null || true)
    # Ensure it's a single integer
    CLIPPY_WARNINGS=$(echo "$CLIPPY_WARNINGS" | tr -d '[:space:]')
    [ -z "$CLIPPY_WARNINGS" ] && CLIPPY_WARNINGS=0
    if [ "$CLIPPY_WARNINGS" -gt 0 ] || [ "$CLIPPY_OK" = false ]; then
        CLIPPY_ERRORS=$(echo "$CLIPPY_OUTPUT" | grep -B1 "^warning\[" | head -20)
    fi
else
    CLIPPY_OK=false
    CLIPPY_ERRORS="skipped (cargo check failed)"
fi

# --- Custom checks ---
CUSTOM_ISSUES=""

# Check 1: broken standalone serve() in lib.rs
if grep -n "pub async fn serve" rust_squared/src/lib.rs 2>/dev/null | grep -v "impl\|self" > /dev/null 2>&1; then
    CUSTOM_ISSUES="${CUSTOM_ISSUES}\nBROKEN_SERVE: standalone serve() found in lib.rs (must be method on RsqApp)"
fi

# Check 2: duplicate Cargo.toml keys
for toml in $(find . -name "Cargo.toml" -not -path "./.git/*"); do
    DUPES=$(grep -oP '^(\w[\w-]*)' "$toml" | sort | uniq -d)
    if [ -n "$DUPES" ]; then
        CUSTOM_ISSUES="${CUSTOM_ISSUES}\nDUPLICATE_KEY: $toml has duplicate keys: $DUPES"
    fi
done

# Check 3: unwrap() in non-test code
UNWRAP_COUNT=$(grep -rn '\.unwrap()' rust_squared/src/ --include="*.rs" 2>/dev/null | grep -v "#\[cfg(test)\]" | grep -v "mod tests" | grep -v "// ok:" | wc -l) || UNWRAP_COUNT=0
if [ "$UNWRAP_COUNT" -gt 5 ]; then
    CUSTOM_ISSUES="${CUSTOM_ISSUES}\nUNWRAP_ABUSE: ${UNWRAP_COUNT} unwrap() calls in non-test code"
fi

# Check 4: TODO/FIXME count
TODO_COUNT=$(grep -rn "TODO\|FIXME" rust_squared/src/ --include="*.rs" 2>/dev/null | wc -l) || TODO_COUNT=0

# --- Build JSON report ---
python3 << PYEOF
import json
from datetime import datetime

def safe_int(s):
    try:
        return int(s.strip().split('\n')[0])
    except (ValueError, IndexError):
        return 0

report = {
    "timestamp": datetime.now().isoformat(),
    "commit": "$COMMIT_FULL",
    "commit_short": "$COMMIT",
    "cargo_check": {
        "pass": "$CHECK_OK" == "true",
        "errors": """$CHECK_ERRORS"""
    },
    "cargo_test": {
        "pass": "$TEST_OK" == "true",
        "passed": safe_int("$TEST_PASSED"),
        "failed": safe_int("$TEST_FAILED"),
        "total": safe_int("$TEST_TOTAL"),
        "errors": """$TEST_ERRORS"""
    },
    "cargo_clippy": {
        "pass": "$CLIPPY_OK" == "true" and safe_int("$CLIPPY_WARNINGS") == 0,
        "warnings": safe_int("$CLIPPY_WARNINGS"),
        "errors": """$CLIPPY_ERRORS"""
    },
    "custom": {
        "broken_serve": "BROKEN_SERVE" in """$CUSTOM_ISSUES""",
        "unwrap_count": safe_int("$UNWRAP_COUNT"),
        "todo_count": safe_int("$TODO_COUNT"),
        "issues": """$CUSTOM_ISSUES""".strip()
    },
}
with open("$REPORT_FILE", "w") as f:
    json.dump(report, f, indent=2)
PYEOF

echo "Ralph: report written to $REPORT_FILE"

# --- Auto-resolve active tasks before dispatching new ones ---
RESOLVED_TASKS=0
if [ "$DISPATCH_AVAILABLE" = true ] && [ -d "$TASKS_DIR/active" ]; then
    # Build a combined report of current issues for resolution checking
    CURRENT_ISSUES="${CHECK_ERRORS}${TEST_ERRORS}${CLIPPY_ERRORS}${CUSTOM_ISSUES}"
    RESOLVED_TASKS=$(auto_resolve_tasks "$CURRENT_ISSUES" "$COMMIT")
    echo "Ralph: auto-resolved ${RESOLVED_TASKS} task(s)"
fi

# --- Hybrid dispatch (task files + webhooks + Discord) ---
DISPATCHED=0

if [ "$CHECK_OK" = true ] && [ "$TEST_OK" = true ] && [ "${CLIPPY_WARNINGS:-0}" -eq 0 ] && [ -z "$CUSTOM_ISSUES" ]; then
    # All clear — Discord-only notification (no task file for success)
    if [ -n "${DISCORD_WEBHOOK_URL:-}" ]; then
        discord_notify "[RALPH] All Clear — FastRust @ ${COMMIT} | check:PASS test:${TEST_PASSED}/${TEST_TOTAL} clippy:0"
    else
        MSG="[RALPH] All Clear — FastRust @ ${COMMIT}
cargo check: PASS | cargo test: PASS (${TEST_PASSED}/${TEST_TOTAL}) | clippy: PASS (0 warnings)"
        discord_send "$PEENSBOT_TOKEN" "$MSG"
    fi
    echo "Ralph: all clear, notified Discord"
else
    if [ "$CHECK_OK" = false ]; then
        FILES=$(echo "$CHECK_OUTPUT" | grep -oP '(?<=--> )[\w/._]+\.rs:\d+' | head -5)
        for fileline in $FILES; do
            FILE=$(echo "$fileline" | cut -d: -f1)
            LINE=$(echo "$fileline" | cut -d: -f2)
            OWNER=$(get_owner "$FILE")
            ERROR_CONTEXT=$(echo "$CHECK_OUTPUT" | grep -A3 "$fileline" | head -4)

            if [ "$DISPATCH_AVAILABLE" = true ]; then
                dispatch_task \
                    "Fix compile error in ${FILE}:${LINE}" \
                    "Compile error at \`${FILE}:${LINE}\`:\n\`\`\`\n${ERROR_CONTEXT}\n\`\`\`\n\nFix this compile error. Run \`cargo check --workspace\` to verify. Then \`cargo test --workspace\`. Commit and push." \
                    "$OWNER" 1 ralph compile "$COMMIT" "${FILE}:${LINE}"
            else
                TOKEN=$(get_dispatch_token "$FILE")
                MSG="[RALPH] Compile Error — commit ${COMMIT}
File: ${FILE}:${LINE} | Owner: ${OWNER}
\`\`\`
${ERROR_CONTEXT}
\`\`\`
Task: Fix this. Run \`cargo check --workspace\` then \`cargo test --workspace\`. Commit and push."
                discord_send "$TOKEN" "$MSG"
            fi
            DISPATCHED=$((DISPATCHED + 1))
        done
    fi

    if [ "$TEST_OK" = false ]; then
        if [ "$DISPATCH_AVAILABLE" = true ]; then
            dispatch_task \
                "Fix test failures (${TEST_FAILED} failed)" \
                "cargo test: ${TEST_PASSED} passed, ${TEST_FAILED} failed\n\`\`\`\n${TEST_ERRORS}\n\`\`\`\n\nFix the failing tests. Run \`cargo test --workspace\` to verify. Commit and push." \
                "PeensBot" 1 ralph test "$COMMIT" ""
        else
            MSG="[RALPH] Test Failures — commit ${COMMIT}
cargo test: ${TEST_PASSED} passed, ${TEST_FAILED} failed
\`\`\`
${TEST_ERRORS}
\`\`\`
Task: Fix the failing tests. Run \`cargo test --workspace\` to verify. Commit and push."
            discord_send "$PEENSBOT_TOKEN" "$MSG"
        fi
        DISPATCHED=$((DISPATCHED + 1))
    fi

    if [ "${CLIPPY_WARNINGS:-0}" -gt 0 ]; then
        FILES=$(echo "$CLIPPY_OUTPUT" | grep -oP '(?<=--> )[\w/._]+\.rs:\d+' | head -5)
        for fileline in $FILES; do
            FILE=$(echo "$fileline" | cut -d: -f1)
            LINE=$(echo "$fileline" | cut -d: -f2)
            OWNER=$(get_owner "$FILE")
            WARN_CONTEXT=$(echo "$CLIPPY_OUTPUT" | grep -B1 -A3 "$fileline" | head -5)

            if [ "$DISPATCH_AVAILABLE" = true ]; then
                dispatch_task \
                    "Fix clippy warning in ${FILE}:${LINE}" \
                    "Clippy warning at \`${FILE}:${LINE}\`:\n\`\`\`\n${WARN_CONTEXT}\n\`\`\`\n\nFix this clippy warning. Run \`cargo clippy --workspace\` to verify. Then \`cargo test --workspace\`. Commit and push." \
                    "$OWNER" 2 ralph clippy "$COMMIT" "${FILE}:${LINE}"
            else
                TOKEN=$(get_dispatch_token "$FILE")
                MSG="[RALPH] Clippy Warning — commit ${COMMIT}
File: ${FILE}:${LINE} | Owner: ${OWNER}
\`\`\`
${WARN_CONTEXT}
\`\`\`
Task: Fix this. Run \`cargo clippy --workspace\` then \`cargo test --workspace\`. Commit and push."
                discord_send "$TOKEN" "$MSG"
            fi
            DISPATCHED=$((DISPATCHED + 1))
        done
    fi

    if echo -e "$CUSTOM_ISSUES" | grep -q "BROKEN_SERVE"; then
        if [ "$DISPATCH_AVAILABLE" = true ]; then
            dispatch_task \
                "CRITICAL: Remove standalone serve() from lib.rs" \
                "Broken standalone \`serve()\` function found in \`lib.rs\`.\nThis has been re-added 3+ times. It MUST be a method on RsqApp, not a standalone function.\n\nRemove the standalone \`pub async fn serve()\` from lib.rs. The serve method belongs on \`impl RsqApp\` in app.rs. Run \`cargo check --workspace\` to verify. Commit and push." \
                "Penemaster" 1 ralph custom "$COMMIT" "rust_squared/src/lib.rs"
        else
            MSG="[RALPH] CRITICAL — commit ${COMMIT}
Broken standalone \`serve()\` in \`lib.rs\`. Must be method on RsqApp.
Task: Remove standalone serve(). Run \`cargo check --workspace\`. Commit and push."
            discord_send "$PENEMASTER_TOKEN" "$MSG"
        fi
        DISPATCHED=$((DISPATCHED + 1))
    fi

    # Summary notification
    if [ "$DISPATCH_AVAILABLE" = true ] && [ -n "${DISCORD_WEBHOOK_URL:-}" ]; then
        QUEUE_COUNT=$(find "$TASKS_DIR/queue" -name "*.md" -not -name ".gitkeep" 2>/dev/null | wc -l)
        ACTIVE_COUNT=$(find "$TASKS_DIR/active" -name "*.md" -not -name ".gitkeep" 2>/dev/null | wc -l)
        discord_notify "[RALPH] @ ${COMMIT} — check:$([ "$CHECK_OK" = true ] && echo PASS || echo FAIL) test:${TEST_PASSED}/${TEST_TOTAL} clippy:${CLIPPY_WARNINGS:-0} | ${DISPATCHED} dispatched, ${QUEUE_COUNT} queued, ${ACTIVE_COUNT} active, ${RESOLVED_TASKS} resolved"
    fi

    echo "Ralph: dispatched $DISPATCHED issues"
fi

# --- Append task tracking to JSON report ---
if [ -f "$REPORT_FILE" ]; then
    python3 << PYEOF2
import json
with open("$REPORT_FILE", "r") as f:
    report = json.load(f)
report["tasks"] = {
    "dispatched": int("${DISPATCHED:-0}"),
    "resolved": int("${RESOLVED_TASKS:-0}")
}
with open("$REPORT_FILE", "w") as f:
    json.dump(report, f, indent=2)
PYEOF2
fi

# --- Cleanup old reports (keep last 100) ---
ls -t "$REPORT_DIR"/report_*.json 2>/dev/null | tail -n +101 | xargs -r rm

echo "Ralph: done"
