from __future__ import annotations

import itertools
import json
import os
import socket
from pathlib import Path
from typing import Any


class HerdrError(RuntimeError):
    """Raised when Herdr rejects or cannot complete a socket request."""


class HerdrClient:
    """Small newline-delimited JSON client for Herdr's Unix socket API."""

    def __init__(self, socket_path: str | os.PathLike[str] | None = None) -> None:
        configured = socket_path or os.environ.get("HERDR_SOCKET_PATH")
        if not configured:
            raise HerdrError("HERDR_SOCKET_PATH is not available")
        self.socket_path = Path(configured)
        self._ids = itertools.count(1)

    def request(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        request_id = f"panemorph:{os.getpid()}:{next(self._ids)}"
        payload = {
            "id": request_id,
            "method": method,
            "params": params or {},
        }
        encoded = json.dumps(payload, separators=(",", ":")).encode("utf-8") + b"\n"

        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
                connection.settimeout(10)
                connection.connect(str(self.socket_path))
                connection.sendall(encoded)
                with connection.makefile("rb") as stream:
                    line = stream.readline()
        except (OSError, TimeoutError) as error:
            raise HerdrError(f"Cannot reach Herdr: {error}") from error

        if not line:
            raise HerdrError("Herdr closed the socket without a response")
        try:
            response = json.loads(line)
        except json.JSONDecodeError as error:
            raise HerdrError("Herdr returned invalid JSON") from error
        if response.get("id") != request_id:
            raise HerdrError("Herdr returned a response for a different request")
        if "error" in response:
            detail = response["error"]
            if isinstance(detail, dict):
                message = detail.get("message") or detail.get("code") or str(detail)
            else:
                message = str(detail)
            raise HerdrError(message)
        result = response.get("result")
        if not isinstance(result, dict):
            raise HerdrError("Herdr response did not contain an object result")
        return result

    def snapshot(self) -> dict[str, Any]:
        result = self.request("session.snapshot")
        snapshot = result.get("snapshot")
        if not isinstance(snapshot, dict):
            raise HerdrError("Herdr snapshot is missing")
        return snapshot

    def current_pane(self, caller_pane_id: str) -> dict[str, Any]:
        result = self.request("pane.current", {"caller_pane_id": caller_pane_id})
        pane = result.get("pane")
        if not isinstance(pane, dict):
            raise HerdrError("The calling pane no longer exists")
        return pane

    def notify(self, title: str, body: str = "") -> None:
        try:
            self.request(
                "notification.show",
                {"title": title, "body": body, "sound": "none"},
            )
        except HerdrError:
            # Notifications are best effort and may be disabled by the user.
            return


def caller_pane_id() -> str:
    pane_id = os.environ.get("PANEMORPH_SOURCE_PANE_ID") or os.environ.get("HERDR_PANE_ID")
    if pane_id:
        return pane_id

    context = os.environ.get("HERDR_PLUGIN_CONTEXT_JSON", "")
    if context:
        try:
            payload = json.loads(context)
        except json.JSONDecodeError:
            payload = {}
        pane_id = payload.get("focused_pane_id")
        if isinstance(pane_id, str) and pane_id:
            return pane_id
    raise HerdrError("paneMorph was not invoked from a Herdr pane")

