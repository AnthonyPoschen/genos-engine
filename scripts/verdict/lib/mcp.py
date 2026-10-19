"""Loopback MCP command client. Quit is `POST /cmd/quit`."""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.request

from lib.contract import Broken


def post(port: int, name: str, payload: dict, timeout: float = 120) -> dict:
    body = json.dumps(payload).encode()
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}/cmd/{name}",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            data = json.loads(response.read().decode())
    except urllib.error.URLError as err:
        raise Broken(f"mcp {name} failed: {err}") from err
    if not isinstance(data, dict):
        raise Broken(f"mcp {name} reply was not an object")
    return data


def wait_ready(port: int, proc, timeout: float = 30) -> None:
    deadline = time.monotonic() + timeout
    url = f"http://127.0.0.1:{port}/cmd"
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise Broken(f"app exited {proc.returncode} before mcp answered")
        try:
            with urllib.request.urlopen(url, timeout=1):
                return
        except urllib.error.URLError:
            time.sleep(0.2)
    raise Broken("mcp did not answer")


def quit(port: int) -> None:
    post(port, "quit", {"code": 0}, timeout=5)
