#!/bin/bash
# Bot commit review script
# Pulls latest from GitHub and shows what the bots pushed

REPO="/c/Users/thegr/OneDrive/Desktop/Purabh/FastRust"
PEENS_KEY="$HOME/.ssh/penemaster"
PEENS_HOST="root@167.71.82.90"
PENE_HOST="root@68.183.151.76"
CHANNEL="1479674568410136719"
PEENSBOT_TOKEN="<SET_LOCALLY>"

cd "$REPO" || exit 1

BEFORE=$(git rev-parse HEAD)
git pull origin main --quiet 2>/dev/null
AFTER=$(git rev-parse HEAD)

if [ "$BEFORE" = "$AFTER" ]; then
    echo "No new commits from bots."
    exit 0
fi

echo "=== NEW COMMITS ==="
git log --oneline "$BEFORE..$AFTER"
echo ""
echo "=== DIFF STATS ==="
git diff --stat "$BEFORE..$AFTER"
echo ""
echo "=== FULL DIFF ==="
git diff "$BEFORE..$AFTER"
echo ""

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
