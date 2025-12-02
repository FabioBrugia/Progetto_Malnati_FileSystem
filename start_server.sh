#!/bin/bash
# Script per avviare il server del Remote File System

echo "=== Remote File System Server (Rust) ==="
echo "Avvio del server sulla porta 8080..."
echo ""

cd "$(dirname "$0")"
cargo run --bin server

