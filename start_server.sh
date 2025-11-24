#!/bin/bash
# Script per avviare il server del Remote File System

echo "=== Remote File System Server ==="
echo "Avvio del server sulla porta 8080..."
echo ""

cd "$(dirname "$0")"
python3 test_server.py

