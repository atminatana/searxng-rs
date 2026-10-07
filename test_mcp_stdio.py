#!/usr/bin/env python3
"""MCP stdio health check - stdlib only.

Spawns `searxng-rs mcp`, runs initialize + a live `search` call and verifies
that every stdout line is a JSON-RPC message (the MCP spec forbids anything
else on a stdio server's stdout; logs belong on stderr).

Usage:
    python test_mcp_stdio.py [--exe PATH] [--config PATH] [--engines bing]
                             [--query "test query"] [--language ru]

Exit code: 0 - search returned results and stdout was clean; 1 - otherwise.
"""
import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time

PROJECT_DIR = os.path.dirname(os.path.abspath(__file__))
EXE_NAME = "searxng-rs.exe" if sys.platform == "win32" else "searxng-rs"
DEFAULT_EXE = os.path.join(PROJECT_DIR, "target", "debug", EXE_NAME)
RESPONSE_TIMEOUT = 20


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", default=DEFAULT_EXE, help=f"server binary (default: {DEFAULT_EXE})")
    parser.add_argument("--config", help="TOML config passed via --config")
    parser.add_argument("--engines", default="bing", help="comma-separated engines (default: bing)")
    parser.add_argument("--query", default="test query")
    parser.add_argument("--language", default="ru")
    return parser.parse_args()


class StdioServer:
    def __init__(self, exe, config):
        # NOTE: Windows CreateProcess does not resolve relative paths with "/".
        command = [os.path.abspath(exe)] + (["--config", config] if config else []) + ["mcp"]
        print(f"$ {' '.join(command)}")
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.stdout_lines = queue.Queue()
        self.stderr_lines = []
        self.non_json_lines = []
        threading.Thread(target=self._pump, args=(self.process.stdout, self.stdout_lines.put), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.process.stderr, self.stderr_lines.append), daemon=True).start()

    @staticmethod
    def _pump(stream, sink):
        for line in iter(stream.readline, b""):
            sink(line)

    def send(self, message):
        self.process.stdin.write((json.dumps(message) + "\n").encode("utf-8"))
        self.process.stdin.flush()

    def receive_response(self, request_id):
        deadline = time.time() + RESPONSE_TIMEOUT
        while time.time() < deadline:
            try:
                line = self.stdout_lines.get(timeout=0.5)
            except queue.Empty:
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                self.non_json_lines.append(line)
                print(f"FAIL: non-JSON line on stdout: {line[:160]!r}")
                continue
            if message.get("id") == request_id:
                return message
        print(f"FAIL: no response with id={request_id} within {RESPONSE_TIMEOUT}s")
        return None

    def stop(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()


def run_search(server, args):
    server.send({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "search",
        "arguments": {"query": args.query, "engines": args.engines, "language": args.language},
    }})
    response = server.receive_response(2)
    if response is None:
        return 0
    if "error" in response:
        print(f"FAIL: JSON-RPC error: {response['error']}")
        return 0
    text = response["result"]["content"][0]["text"]
    if not text.startswith("["):
        print(f"FAIL: search returned: {text!r}")
        return 0
    results = json.loads(text)
    for result in results[:3]:
        print(f"  - {result['title']}  <{result['url']}>")
    return len(results)


def main():
    args = parse_args()
    server = StdioServer(args.exe, args.config)
    try:
        server.send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mcp-stdio-healthcheck", "version": "1.0"},
        }})
        if server.receive_response(1) is None:
            return 1
        print("OK: initialize")
        server.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        result_count = run_search(server, args)
        print(f"search ({args.engines}): {result_count} results")
    finally:
        server.stop()

    print(f"non-JSON stdout lines: {len(server.non_json_lines)} | stderr (log) lines: {len(server.stderr_lines)}")
    is_ok = result_count > 0 and not server.non_json_lines
    print("=== Check complete: OK ===" if is_ok else "=== Check complete: FAIL ===")
    return 0 if is_ok else 1


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main())
