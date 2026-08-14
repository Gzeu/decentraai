#!/usr/bin/env python3
# Minimal mock OpenAI-compatible SSE server using standard library
# Handles GET /health and POST /v1/chat/completions (streaming via chunked responses)

import json
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
import socketserver
import threading
import sys

TOKENS = ["Hello", " world", "!\n"]

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def do_GET(self):
        if self.path == '/health':
            self.send_response(200)
            self.send_header('Content-Type', 'text/plain')
            self.send_header('Content-Length', '2')
            self.end_headers()
            self.wfile.write(b'ok')
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path == '/v1/chat/completions':
            # Read body (Content-Length)
            length = int(self.headers.get('Content-Length', 0))
            if length:
                _ = self.rfile.read(length)
            # Start chunked response for SSE
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Transfer-Encoding', 'chunked')
            self.end_headers()
            try:
                for t in TOKENS:
                    payload = {"choices":[{"delta":{"content": t}, "finish_reason": None}]}
                    chunk = f"data: {json.dumps(payload)}\n\n".encode('utf-8')
                    # Write chunk length in hex + CRLF
                    self.wfile.write(b"%x\r\n" % len(chunk))
                    self.wfile.write(chunk)
                    self.wfile.write(b"\r\n")
                    self.wfile.flush()
                    time.sleep(0.25)
                done = b"data: [DONE]\n\n"
                self.wfile.write(b"%x\r\n" % len(done))
                self.wfile.write(done)
                self.wfile.write(b"\r\n")
                # Final zero-length chunk
                self.wfile.write(b"0\r\n\r\n")
                self.wfile.flush()
            except BrokenPipeError:
                pass
        else:
            self.send_response(404)
            self.end_headers()

class ThreadedHTTPServer(socketserver.ThreadingMixIn, HTTPServer):
    daemon_threads = True

def run_server(port=8081):
    server = ThreadedHTTPServer(('127.0.0.1', port), Handler)
    print(f"Mock llama-server (stdlib) running on http://127.0.0.1:{port}")
    server.serve_forever()

if __name__ == '__main__':
    port = 8081
    if len(sys.argv) > 1:
        try:
            port = int(sys.argv[1])
        except:
            pass
    run_server(port)
