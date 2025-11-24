#!/usr/bin/env python3
"""
Remote File System Server - RESTful API
Implements the server-side API for remote file system operations.
"""

from flask import Flask, jsonify, request, send_file
import os
import shutil
from pathlib import Path
from datetime import datetime
import json

app = Flask(__name__)

# Base directory for remote file storage
BASE_DIR = Path(__file__).parent / "server_storage"
BASE_DIR.mkdir(exist_ok=True)


def get_safe_path(path):
    """Ensure the path is within BASE_DIR to prevent directory traversal attacks."""
    requested_path = BASE_DIR / path.lstrip('/')
    try:
        requested_path = requested_path.resolve()
        BASE_DIR.resolve()
        if not str(requested_path).startswith(str(BASE_DIR.resolve())):
            return None
        return requested_path
    except:
        return None


@app.route('/list/<path:path>', methods=['GET'])
@app.route('/list/', defaults={'path': ''}, methods=['GET'])
def list_directory(path):
    """List directory contents."""
    dir_path = get_safe_path(path)

    if dir_path is None:
        return jsonify({'error': 'Invalid path'}), 400

    if not dir_path.exists():
        return jsonify({'error': 'Directory not found'}), 404

    if not dir_path.is_dir():
        return jsonify({'error': 'Not a directory'}), 400

    try:
        entries = []
        for entry in dir_path.iterdir():
            stat = entry.stat()
            entries.append({
                'name': entry.name,
                'is_dir': entry.is_dir(),
                'size': stat.st_size if entry.is_file() else 0,
                'mtime': stat.st_mtime,
                'ctime': stat.st_ctime,
                'mode': stat.st_mode,
            })

        return jsonify({'entries': entries}), 200
    except Exception as e:
        return jsonify({'error': str(e)}), 500


@app.route('/files/<path:path>', methods=['GET'])
def read_file(path):
    """Read file contents."""
    file_path = get_safe_path(path)

    if file_path is None:
        return jsonify({'error': 'Invalid path'}), 400

    if not file_path.exists():
        return jsonify({'error': 'File not found'}), 404

    if not file_path.is_file():
        return jsonify({'error': 'Not a file'}), 400

    try:
        return send_file(file_path, as_attachment=False)
    except Exception as e:
        return jsonify({'error': str(e)}), 500


@app.route('/files/<path:path>', methods=['PUT'])
def write_file(path):
    """Write file contents."""
    file_path = get_safe_path(path)

    if file_path is None:
        return jsonify({'error': 'Invalid path'}), 400

    try:
        # Create parent directories if they don't exist
        file_path.parent.mkdir(parents=True, exist_ok=True)

        # Write the file
        with open(file_path, 'wb') as f:
            f.write(request.data)

        stat = file_path.stat()
        return jsonify({
            'success': True,
            'size': stat.st_size,
            'mtime': stat.st_mtime
        }), 200
    except Exception as e:
        return jsonify({'error': str(e)}), 500


@app.route('/mkdir/<path:path>', methods=['POST'])
def create_directory(path):
    """Create directory."""
    dir_path = get_safe_path(path)

    if dir_path is None:
        return jsonify({'error': 'Invalid path'}), 400

    try:
        dir_path.mkdir(parents=True, exist_ok=True)
        return jsonify({'success': True}), 200
    except Exception as e:
        return jsonify({'error': str(e)}), 500


@app.route('/files/<path:path>', methods=['DELETE'])
def delete_file(path):
    """Delete file or directory."""
    target_path = get_safe_path(path)

    if target_path is None:
        return jsonify({'error': 'Invalid path'}), 400

    if not target_path.exists():
        return jsonify({'error': 'File or directory not found'}), 404

    try:
        if target_path.is_dir():
            shutil.rmtree(target_path)
        else:
            target_path.unlink()

        return jsonify({'success': True}), 200
    except Exception as e:
        return jsonify({'error': str(e)}), 500


@app.route('/rename', methods=['POST'])
def rename_file():
    """Rename or move file/directory."""
    data = request.get_json()

    if not data or 'from' not in data or 'to' not in data:
        return jsonify({'error': 'Missing from or to path'}), 400

    from_path = get_safe_path(data['from'])
    to_path = get_safe_path(data['to'])

    if from_path is None or to_path is None:
        return jsonify({'error': 'Invalid path'}), 400

    if not from_path.exists():
        return jsonify({'error': 'Source not found'}), 404

    try:
        # Create parent directory if it doesn't exist
        to_path.parent.mkdir(parents=True, exist_ok=True)
        from_path.rename(to_path)
        return jsonify({'success': True}), 200
    except Exception as e:
        return jsonify({'error': str(e)}), 500


@app.route('/health', methods=['GET'])
def health_check():
    """Health check endpoint."""
    return jsonify({'status': 'ok'}), 200


if __name__ == '__main__':
    print(f"Starting Remote File System Server")
    print(f"Storage directory: {BASE_DIR.absolute()}")

    # Create some test files
    test_dir = BASE_DIR / "test"
    test_dir.mkdir(exist_ok=True)
    (test_dir / "hello.txt").write_text("Hello from remote file system!")

    app.run(host='0.0.0.0', port=8080, debug=True)

