from __future__ import annotations

import os
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from panemorph.api import HerdrClient, HerdrError, caller_pane_id
from panemorph.service import PaneMorphService
from panemorph.model import Choice


def selector_command(herdr: str, plugin_id: str, source_pane_id: str, mode: str,
                     result_path: str = "") -> list[str]:
    return [
        herdr,
        "plugin",
        "pane",
        "open",
        "--plugin",
        plugin_id,
        "--entrypoint",
        "selector",
        "--placement",
        "overlay",
        "--env",
        f"PANEMORPH_MODE={mode}",
        "--env",
        f"PANEMORPH_SOURCE_PANE_ID={source_pane_id}",
        "--env",
        f"PANEMORPH_RESULT_PATH={result_path}",
        "--focus",
    ]


def finish_selection(client: HerdrClient, source_pane_id: str, selector_pane_id: str,
                     result_path: Path, mode: str) -> None:
    # Herdr implements an overlay as a temporary zoomed pane. Wait for its
    # process to exit and Herdr to restore the layout before moving live panes.
    while any(pane.get("pane_id") == selector_pane_id
              for pane in client.snapshot().get("panes", [])):
        time.sleep(0.15)
    if not result_path.exists():
        raise HerdrError("The selector exited without a result; check its terminal error")
    result = json.loads(result_path.read_text())
    if "error" in result:
        raise HerdrError(result["error"])
    if result.get("choice") is None:
        return
    choice = Choice(**result["choice"])
    service = PaneMorphService(client)
    source = service.current(source_pane_id)
    if mode == "send":
        service.send(source, choice)
    else:
        service.bring(source, choice)


def main(argv: list[str] | None = None) -> int:
    args = argv if argv is not None else sys.argv[1:]
    if len(args) != 1 or args[0] not in {"send", "bring"}:
        print("usage: open_selector.py send|bring", file=sys.stderr)
        return 2
    mode = args[0]
    try:
        client = HerdrClient()
        service = PaneMorphService(client)
        source = service.current(caller_pane_id())
        source_pane_id = str(source["pane_id"])
        herdr = os.environ.get("HERDR_BIN_PATH", "herdr")
        with tempfile.TemporaryDirectory(prefix="panemorph-") as directory:
            result_path = Path(directory) / "selection.json"
            command = selector_command(
                herdr,
                os.environ.get("HERDR_PLUGIN_ID", "dev.panemorph"),
                source_pane_id,
                mode,
                str(result_path),
            )
            completed = subprocess.run(command, check=False, capture_output=True, text=True)
            if completed.returncode:
                message = completed.stderr.strip() or completed.stdout.strip() or "selector failed"
                raise HerdrError(message)
            opened = json.loads(completed.stdout)
            selector_id = opened["result"]["plugin_pane"]["pane"]["pane_id"]
            finish_selection(client, source_pane_id, selector_id, result_path, mode)
    except (HerdrError, OSError, ValueError, KeyError, TypeError) as error:
        try:
            client.notify("paneMorph could not complete the move", str(error))
        except UnboundLocalError:
            pass
        print(f"paneMorph: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
