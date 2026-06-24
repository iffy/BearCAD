# LE3 issue monitor

Scripts for watching [iffy/LE3](https://github.com/iffy/LE3) issues labeled `doing` and autonomously fixing them on `devel`.

## Usage

```bash
# Run two poll cycles (verification)
MAX_CYCLES=2 POLL_INTERVAL=5 ./scripts/le3-monitor.sh

# Start persistent monitor (single instance via PID lock)
./scripts/le3-monitor.sh

# Handle one issue directly
./scripts/le3-issue-handler.sh 3
```

Runtime state (logs, checkpoint, PID) is stored in `scripts/monitor-state/` (gitignored).