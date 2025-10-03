#!/usr/bin/env python3
# filepath: /home/fabiobrugiafreddo/RustroverProjects/Progetto_Malnati_FileSystem/test_server.py
"""
Server di test per il Remote File System
Implementa le API REST specificate nelle specifiche del progetto
"""

from flask import Flask, request, jsonify, send_file
import os
import json
from datetime import datetime
import tempfile
import shutil

app = Flask(__name__)

# Directory di base per il server (simulazione del filesystem remoto)
BASE_DIR = "/tmp/remote_fs_test"

def init_test_data():
    """Inizializza alcuni file e directory di test"""
    if os.path.exists(BASE_DIR):
        shutil.rmtree(BASE_DIR)
    os.makedirs(BASE_DIR, exist_ok=True)

    # Crea alcuni file di test
    with open(os.path.join(BASE_DIR, "test.txt"), "w") as f:
        f.write("Questo è un file di test per il filesystem remoto!\n")

    with open(os.path.join(BASE_DIR, "hello.txt"), "w") as f:
        f.write("Hello, World!\n")

    # Crea una directory di test
    os.makedirs(os.path.join(BASE_DIR, "test_dir"), exist_ok=True)
    with open(os.path.join(BASE_DIR, "test_dir", "nested.txt"), "w") as f:
        f.write("File dentro una directory\n")

def get_full_path(path):
    """Converte un path relativo in path assoluto sicuro"""
    if path.startswith('/'):
        path = path[1:]
    full_path = os.path.join(BASE_DIR, path)
    # Verifica che il path sia dentro BASE_DIR (sicurezza)
    if not os.path.abspath(full_path).startswith(os.path.abspath(BASE_DIR)):
        return None
    return full_path

@app.route('/list/<path:directory_path>')
@app.route('/list/', defaults={'directory_path': ''})
def list_directory(directory_path):
    """GET /list/<path> – List directory contents"""
    full_path = get_full_path(directory_path)
    if not full_path or not os.path.exists(full_path):
        return jsonify({"error": "Directory not found"}), 404

    if not os.path.isdir(full_path):
        return jsonify({"error": "Not a directory"}), 400

    entries = []
    try:
        for item in os.listdir(full_path):
            item_path = os.path.join(full_path, item)
            entries.append({
                "name": item,
                "isDirectory": os.path.isdir(item_path)
            })
        return jsonify(entries)
    except Exception as e:
        return jsonify({"error": str(e)}), 500

@app.route('/files/<path:file_path>', methods=['GET'])
def read_file(file_path):
    """GET /files/<path> – Read file contents"""
    full_path = get_full_path(file_path)
    if not full_path or not os.path.exists(full_path):
        return jsonify({"error": "File not found"}), 404

    if os.path.isdir(full_path):
        return jsonify({"error": "Is a directory"}), 400

    try:
        # Supporto per Range requests (lettura parziale)
        range_header = request.headers.get('Range')
        if range_header:
            # Parsing semplificato del range header
            range_match = range_header.replace('bytes=', '').split('-')
            start = int(range_match[0]) if range_match[0] else 0

            with open(full_path, 'rb') as f:
                f.seek(start)
                if len(range_match) > 1 and range_match[1]:
                    end = int(range_match[1])
                    content = f.read(end - start + 1)
                else:
                    content = f.read()

                response = app.response_class(
                    content,
                    206,  # Partial Content
                    headers={
                        'Content-Range': f'bytes {start}-{start + len(content) - 1}/{os.path.getsize(full_path)}',
                        'Content-Length': str(len(content)),
                        'Last-Modified': datetime.fromtimestamp(os.path.getmtime(full_path)).strftime('%a, %d %b %Y %H:%M:%S GMT')
                    }
                )
                return response
        else:
            # Lettura completa del file
            return send_file(full_path)

    except Exception as e:
        return jsonify({"error": str(e)}), 500

@app.route('/files/<path:file_path>', methods=['PUT'])
def write_file(file_path):
    """PUT /files/<path> – Write file contents"""
    full_path = get_full_path(file_path)
    if not full_path:
        return jsonify({"error": "Invalid path"}), 400

    try:
        # Crea le directory parent se necessarie
        os.makedirs(os.path.dirname(full_path), exist_ok=True)

        with open(full_path, 'wb') as f:
            f.write(request.get_data())

        return jsonify({"success": True, "bytes_written": len(request.get_data())})
    except Exception as e:
        return jsonify({"error": str(e)}), 500

@app.route('/mkdir/<path:dir_path>', methods=['POST'])
def create_directory(dir_path):
    """POST /mkdir/<path> – Create directory"""
    full_path = get_full_path(dir_path)
    if not full_path:
        return jsonify({"error": "Invalid path"}), 400

    try:
        os.makedirs(full_path, exist_ok=True)
        return jsonify({"success": True})
    except Exception as e:
        return jsonify({"error": str(e)}), 500

@app.route('/files/<path:file_path>', methods=['DELETE'])
def delete_file(file_path):
    """DELETE /files/<path> – Delete file or directory"""
    full_path = get_full_path(file_path)
    if not full_path or not os.path.exists(full_path):
        return jsonify({"error": "File not found"}), 404

    try:
        if os.path.isdir(full_path):
            shutil.rmtree(full_path)
        else:
            os.remove(full_path)
        return jsonify({"success": True})
    except Exception as e:
        return jsonify({"error": str(e)}), 500

# HEAD requests per ottenere metadata dei file
@app.route('/files/<path:file_path>', methods=['HEAD'])
def file_info(file_path):
    """HEAD /files/<path> – Get file metadata"""
    full_path = get_full_path(file_path)
    if not full_path or not os.path.exists(full_path):
        return '', 404

    if os.path.isdir(full_path):
        return '', 400

    try:
        stat = os.stat(full_path)
        response = app.response_class()
        response.headers['Content-Length'] = str(stat.st_size)
        response.headers['Last-Modified'] = datetime.fromtimestamp(stat.st_mtime).strftime('%a, %d %b %Y %H:%M:%S GMT')
        return response
    except Exception as e:
        return '', 500

@app.route('/')
def index():
    """Root endpoint con informazioni sul server"""
    return jsonify({
        "name": "Remote File System Test Server",
        "version": "1.0.0",
        "base_directory": BASE_DIR,
        "endpoints": [
            "GET /list/<path>",
            "GET /files/<path>",
            "PUT /files/<path>",
            "POST /mkdir/<path>",
            "DELETE /files/<path>",
            "HEAD /files/<path>"
        ]
    })

if __name__ == '__main__':
    print("Inizializzazione server di test...")
    init_test_data()
    print(f"Directory base del server: {BASE_DIR}")
    print("File di test creati:")
    for root, dirs, files in os.walk(BASE_DIR):
        level = root.replace(BASE_DIR, '').count(os.sep)
        indent = ' ' * 2 * level
        print(f"{indent}{os.path.basename(root)}/")
        subindent = ' ' * 2 * (level + 1)
        for file in files:
            print(f"{subindent}{file}")

    print("\nAvvio server su http://localhost:9000")
    print("Premi Ctrl+C per fermare il server")

    app.run(host='0.0.0.0', port=9000, debug=False)
