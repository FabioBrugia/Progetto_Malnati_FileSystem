#!/bin/bash
# Script per avviare il client del Remote File System

MOUNTPOINT="${1:-/tmp/remotefs}"
SERVER="${2:-http://localhost:8080}"
MODE="${3:-deamon}"  # foreground, daemon, stop

echo "=== Remote File System Client ==="
echo "Server: $SERVER"
echo "Mount point: $MOUNTPOINT"
echo ""

# Compila il client
cd "$(dirname "$0")/clientFS"
cargo build --release 2>/dev/null

case "$MODE" in
    daemon|background|bg)
        echo "Avvio in modalità daemon (background)..."
        ./target/release/clientFS --server "$SERVER" --mountpoint "$MOUNTPOINT" --daemon
        echo "Client avviato in background."
        echo "Per vedere i log: tail -f /tmp/clientfs.log"
        echo "Per fermarlo: $0 $MOUNTPOINT $SERVER stop"
        ;;
    stop)
        echo "Fermo il daemon..."
        ./target/release/clientFS --mountpoint "$MOUNTPOINT" --stop
        ;;
    *)
        echo "Avvio in modalità foreground (premi Ctrl+C per smontare)..."
        ./target/release/clientFS --server "$SERVER" --mountpoint "$MOUNTPOINT" --verbose
        ;;
esac

