#!/bin/bash
# Script per avviare il client del Remote File System

MOUNTPOINT="${1:-/tmp/remotefs}"
SERVER="${2:-http://localhost:8080}"

echo "=== Remote File System Client ==="
echo "Server: $SERVER"
echo "Mount point: $MOUNTPOINT"
echo ""

# Crea il mount point se non esiste
mkdir -p "$MOUNTPOINT"

# Compila e avvia il client
cd "$(dirname "$0")/clientFS"
cargo run --release -- --server "$SERVER" --mountpoint "$MOUNTPOINT" --verbose

