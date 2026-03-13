#!/bin/bash
# Bot commit review + task oversight script
# Usage: review_bots.sh [subcommand]
#   (default)  — pull, diffs, remote compile, Ralph report
#   tasks      — show all tasks grouped by status
#   active     — show active tasks only
#   failed     — show failed tasks
#   retry ID   — re-dispatch a failed task (or --all)
#   pause      — pause task dispatch (create sentinel)
#   resume     — resume task dispatch (remove sentinel)
#   health     — query both droplet /status endpoints

REPO="/c/Users/thegr/OneDrive/Desktop/Purabh/FastRust"
PEENS_KEY="$HOME/.ssh/penemaster"
PEENS_HOST="root@167.71.82.90"
PENE_HOST="root@68.183.151.76"
CHANNEL="1479674568410136719"
PEENSBOT_TOKEN="<SET_LOCALLY>"

TASKS_DIR="${REPO}/.tasks"
PEENSBOT_URL="http://167.71.82.90:18790"
PENEMASTER_URL="http://68.183.151.76:18790"

# --- Subcommand routing ---
SUBCMD="${1:-default}"

case "$SUBCMD" in

# =============================================================================
# DEFAULT: Original review behavior (pull, diffs, compile check, Ralph report)
# =============================================================================
default)
    cd "$REPO" || exit 1

    BEFORE=$(git rev-parse HEAD)
    git pull origin main --quiet 2>/dev/null
    AFTER=$(git rev-parse HEAD)

    if [ "$BEFORE" = "$AFTER" ]; then
        echo "No new commits from bots."
    else
        echo "=== NEW COMMITS ==="
        git log --oneline "$BEFORE..$AFTER"
        echo ""
        echo "=== DIFF STATS ==="
        git diff --stat "$BEFORE..$AFTER"
        echo ""
        echo "=== FULL DIFF ==="
        git diff "$BEFORE..$AFTER"
        echo ""
    fi

    # Check if it compiles on PEENS
    echo "=== REMOTE COMPILE CHECK (PEENS) ==="
    ssh -o ConnectTimeout=10 -i "$PEENS_KEY" "$PEENS_HOST" \
        "cd /home/openclaw/.openclaw/workspace/FastRust && git pull origin main --quiet 2>/dev/null && sudo -u openclaw bash -c 'source /home/openclaw/.cargo/env && cargo check 2>&1 | tail -5 && cargo test 2>&1 | tail -10'" 2>&1

    # Ralph quality report
    echo ""
    echo "=== RALPH QUALITY REPORT ==="
    LATEST_REPORT=$(ssh -o ConnectTimeout=10 -i "$PEENS_KEY" "$PEENS_HOST" \
        "ls -t /opt/ralph/reports/report_*.json 2>/dev/null | head -1")
    if [ -n "$LATEST_REPORT" ]; then
        ssh -o ConnectTimeout=10 -i "$PEENS_KEY" "$PEENS_HOST" "cat $LATEST_REPORT" 2>&1
    else
        echo "No Ralph reports found. Run: ssh -i $PEENS_KEY $PEENS_HOST 'bash /opt/ralph/ralph.sh'"
    fi

    # Show task summary if tasks dir exists
    if [ -d "$TASKS_DIR" ]; then
        echo ""
        echo "=== TASK SUMMARY ==="
        for status in queue active done failed; do
            dir="$TASKS_DIR/$status"
            count=0
            if [ -d "$dir" ]; then
                count=$(find "$dir" -name "*.md" -not -name ".gitkeep" 2>/dev/null | wc -l)
            fi
            printf "  %-8s %d\n" "$status:" "$count"
        done
        if [ -f "$TASKS_DIR/.paused" ]; then
            echo "  ** DISPATCH PAUSED **"
        fi
    fi
    ;;

# =============================================================================
# TASKS: Show all tasks grouped by status
# =============================================================================
tasks)
    cd "$REPO" 2>/dev/null && git pull origin main --quiet 2>/dev/null

    if [ ! -d "$TASKS_DIR" ]; then
        echo "No .tasks/ directory found."
        exit 1
    fi

    if [ -f "$TASKS_DIR/.paused" ]; then
        echo "** DISPATCH IS PAUSED **"
        echo ""
    fi

    for status in queue active failed done; do
        dir="$TASKS_DIR/$status"
        echo "=== ${status^^} ==="
        if [ ! -d "$dir" ]; then
            echo "  (no directory)"
            echo ""
            continue
        fi

        found=0
        for f in "$dir"/*.md; do
            [ -f "$f" ] || continue
            [ "$(basename $f)" = ".gitkeep" ] && continue
            found=1

            title=$(grep "^title:" "$f" 2>/dev/null | head -1 | sed 's/title: *"//' | sed 's/"$//')
            assignee=$(grep "^assignee:" "$f" 2>/dev/null | head -1 | sed 's/assignee: *//')
            priority=$(grep "^priority:" "$f" 2>/dev/null | head -1 | sed 's/priority: *//')
            created=$(grep "^created:" "$f" 2>/dev/null | head -1 | sed 's/created: *"//' | sed 's/"$//')
            check_type=$(grep "^check_type:" "$f" 2>/dev/null | head -1 | sed 's/check_type: *//')
            dispatch_count=$(grep "^dispatch_count:" "$f" 2>/dev/null | head -1 | sed 's/dispatch_count: *//')

            echo "  [P${priority}] ${title}"
            echo "       Assignee: ${assignee} | Type: ${check_type} | Created: ${created} | Attempts: ${dispatch_count}"
            echo "       File: $(basename $f)"
            echo ""
        done

        if [ "$found" -eq 0 ]; then
            echo "  (empty)"
        fi
        echo ""
    done
    ;;

# =============================================================================
# ACTIVE: Show active tasks only
# =============================================================================
active)
    cd "$REPO" 2>/dev/null && git pull origin main --quiet 2>/dev/null
    dir="$TASKS_DIR/active"

    echo "=== ACTIVE TASKS ==="
    if [ ! -d "$dir" ]; then
        echo "  (no active directory)"
        exit 0
    fi

    found=0
    for f in "$dir"/*.md; do
        [ -f "$f" ] || continue
        [ "$(basename $f)" = ".gitkeep" ] && continue
        found=1

        title=$(grep "^title:" "$f" 2>/dev/null | head -1 | sed 's/title: *"//' | sed 's/"$//')
        assignee=$(grep "^assignee:" "$f" 2>/dev/null | head -1 | sed 's/assignee: *//')
        updated=$(grep "^updated:" "$f" 2>/dev/null | head -1 | sed 's/updated: *"//' | sed 's/"$//')

        echo "  ${title} → ${assignee} (updated: ${updated})"
        echo "    $(basename $f)"
    done

    [ "$found" -eq 0 ] && echo "  (none)"
    ;;

# =============================================================================
# FAILED: Show failed tasks
# =============================================================================
failed)
    cd "$REPO" 2>/dev/null && git pull origin main --quiet 2>/dev/null
    dir="$TASKS_DIR/failed"

    echo "=== FAILED TASKS ==="
    if [ ! -d "$dir" ]; then
        echo "  (no failed directory)"
        exit 0
    fi

    found=0
    for f in "$dir"/*.md; do
        [ -f "$f" ] || continue
        [ "$(basename $f)" = ".gitkeep" ] && continue
        found=1

        title=$(grep "^title:" "$f" 2>/dev/null | head -1 | sed 's/title: *"//' | sed 's/"$//')
        assignee=$(grep "^assignee:" "$f" 2>/dev/null | head -1 | sed 's/assignee: *//')
        dispatch_count=$(grep "^dispatch_count:" "$f" 2>/dev/null | head -1 | sed 's/dispatch_count: *//')

        echo "  ${title} → ${assignee} (attempts: ${dispatch_count})"
        echo "    $(basename $f)"
    done

    [ "$found" -eq 0 ] && echo "  (none)"
    ;;

# =============================================================================
# RETRY: Re-dispatch a failed task
# =============================================================================
retry)
    TASK_ID="${2:-}"
    if [ -z "$TASK_ID" ] && [ "$TASK_ID" != "--all" ]; then
        echo "Usage: review_bots.sh retry <task-filename> | --all"
        exit 1
    fi

    cd "$REPO" 2>/dev/null && git pull origin main --quiet 2>/dev/null
    dir="$TASKS_DIR/failed"

    if [ "$TASK_ID" = "--all" ]; then
        retried=0
        for f in "$dir"/*.md; do
            [ -f "$f" ] || continue
            [ "$(basename $f)" = ".gitkeep" ] && continue
            basename_f=$(basename "$f")
            sed -i "s/^status: failed/status: queue/" "$f"
            sed -i "s/^updated: .*/updated: \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"/" "$f"
            old_count=$(grep "^dispatch_count:" "$f" 2>/dev/null | head -1 | sed 's/dispatch_count: *//')
            new_count=$(( ${old_count:-1} + 1 ))
            sed -i "s/^dispatch_count: .*/dispatch_count: ${new_count}/" "$f"
            mv "$f" "$TASKS_DIR/queue/${basename_f}"
            echo "Retried: ${basename_f} (attempt ${new_count})"
            retried=$((retried + 1))
        done

        if [ "$retried" -gt 0 ]; then
            cd "$REPO"
            git add .tasks/ 2>/dev/null
            git commit -m "[task] Retried ${retried} failed task(s)" --no-gpg-sign 2>/dev/null
            git push origin main 2>/dev/null
            echo "Retried ${retried} task(s)"
        else
            echo "No failed tasks to retry"
        fi
    else
        task_file="$dir/$TASK_ID"
        if [ ! -f "$task_file" ]; then
            echo "Task not found: $task_file"
            exit 1
        fi
        basename_f=$(basename "$task_file")
        sed -i "s/^status: failed/status: queue/" "$task_file"
        sed -i "s/^updated: .*/updated: \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"/" "$task_file"
        old_count=$(grep "^dispatch_count:" "$task_file" 2>/dev/null | head -1 | sed 's/dispatch_count: *//')
        new_count=$(( ${old_count:-1} + 1 ))
        sed -i "s/^dispatch_count: .*/dispatch_count: ${new_count}/" "$task_file"
        mv "$task_file" "$TASKS_DIR/queue/${basename_f}"

        cd "$REPO"
        git add .tasks/ 2>/dev/null
        git commit -m "[task] Retried: ${basename_f}" --no-gpg-sign 2>/dev/null
        git push origin main 2>/dev/null
        echo "Retried: ${basename_f} (attempt ${new_count})"
    fi
    ;;

# =============================================================================
# PAUSE: Create .paused sentinel to block new dispatch
# =============================================================================
pause)
    cd "$REPO" 2>/dev/null && git pull origin main --quiet 2>/dev/null
    mkdir -p "$TASKS_DIR"
    echo "Paused by $(whoami) at $(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$TASKS_DIR/.paused"
    cd "$REPO"
    git add .tasks/.paused 2>/dev/null
    git commit -m "[task] Dispatch paused" --no-gpg-sign 2>/dev/null
    git push origin main 2>/dev/null
    echo "Dispatch PAUSED. TaskBridge will reject new tasks until resumed."
    ;;

# =============================================================================
# RESUME: Remove .paused sentinel
# =============================================================================
resume)
    cd "$REPO" 2>/dev/null && git pull origin main --quiet 2>/dev/null
    if [ -f "$TASKS_DIR/.paused" ]; then
        rm "$TASKS_DIR/.paused"
        cd "$REPO"
        git add .tasks/ 2>/dev/null
        git commit -m "[task] Dispatch resumed" --no-gpg-sign 2>/dev/null
        git push origin main 2>/dev/null
        echo "Dispatch RESUMED."
    else
        echo "Dispatch was not paused."
    fi
    ;;

# =============================================================================
# HEALTH: Query both droplet /status endpoints
# =============================================================================
health)
    echo "=== TASKBRIDGE HEALTH ==="
    echo ""
    echo "--- PEENS (PeensBot) ---"
    PEENS_STATUS=$(curl -s --connect-timeout 5 --max-time 10 "${PEENSBOT_URL}/status" 2>&1)
    if [ $? -eq 0 ] && echo "$PEENS_STATUS" | python3 -m json.tool > /dev/null 2>&1; then
        echo "$PEENS_STATUS" | python3 -m json.tool
    else
        echo "  UNREACHABLE (${PEENSBOT_URL})"
        echo "  Response: ${PEENS_STATUS}"
    fi

    echo ""
    echo "--- Penehouse (Penemaster) ---"
    PENE_STATUS=$(curl -s --connect-timeout 5 --max-time 10 "${PENEMASTER_URL}/status" 2>&1)
    if [ $? -eq 0 ] && echo "$PENE_STATUS" | python3 -m json.tool > /dev/null 2>&1; then
        echo "$PENE_STATUS" | python3 -m json.tool
    else
        echo "  UNREACHABLE (${PENEMASTER_URL})"
        echo "  Response: ${PENE_STATUS}"
    fi

    echo ""
    echo "--- SSH Connectivity ---"
    echo -n "  PEENS:     "
    ssh -o ConnectTimeout=5 -o BatchMode=yes -i "$PEENS_KEY" "$PEENS_HOST" "echo OK" 2>/dev/null || echo "FAILED"
    echo -n "  Penehouse: "
    ssh -o ConnectTimeout=5 -o BatchMode=yes -i "$PEENS_KEY" "$PENE_HOST" "echo OK" 2>/dev/null || echo "FAILED"

    echo ""
    echo "--- OpenClaw Services ---"
    echo -n "  PEENS openclaw:     "
    ssh -o ConnectTimeout=5 -i "$PEENS_KEY" "$PEENS_HOST" "systemctl is-active openclaw 2>/dev/null || echo STOPPED" 2>/dev/null || echo "SSH FAILED"
    echo -n "  Penehouse openclaw: "
    ssh -o ConnectTimeout=5 -i "$PEENS_KEY" "$PENE_HOST" "systemctl is-active openclaw 2>/dev/null || echo STOPPED" 2>/dev/null || echo "SSH FAILED"
    echo -n "  PEENS taskbridge:     "
    ssh -o ConnectTimeout=5 -i "$PEENS_KEY" "$PEENS_HOST" "systemctl is-active taskbridge 2>/dev/null || echo STOPPED" 2>/dev/null || echo "SSH FAILED"
    echo -n "  Penehouse taskbridge: "
    ssh -o ConnectTimeout=5 -i "$PEENS_KEY" "$PENE_HOST" "systemctl is-active taskbridge 2>/dev/null || echo STOPPED" 2>/dev/null || echo "SSH FAILED"
    ;;

# =============================================================================
# HELP
# =============================================================================
help|--help|-h)
    echo "Usage: review_bots.sh [subcommand]"
    echo ""
    echo "Subcommands:"
    echo "  (default)     Pull, diffs, remote compile check, Ralph report"
    echo "  tasks         Show all tasks grouped by status"
    echo "  active        Show active tasks only"
    echo "  failed        Show failed tasks"
    echo "  retry <ID>    Re-dispatch a specific failed task"
    echo "  retry --all   Re-dispatch all failed tasks"
    echo "  pause         Pause task dispatch (creates sentinel file)"
    echo "  resume        Resume task dispatch (removes sentinel)"
    echo "  health        Query both droplet /status endpoints"
    ;;

*)
    echo "Unknown subcommand: $SUBCMD (try 'help')"
    exit 1
    ;;

esac
