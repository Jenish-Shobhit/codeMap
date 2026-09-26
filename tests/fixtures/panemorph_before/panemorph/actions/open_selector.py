from __future__ import annotations

import os
import subprocess
import sys

from panemorph.api import HerdrClient, HerdrError, caller_pane_id
from panemorph.service import PaneMorphService


def selector_command(herdr: str, plugin_id: str, source_pane_id: str, mode: str) -> list[str]:
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
        "--focus",
    ]


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
        command = selector_command(
            herdr,
            os.environ.get("HERDR_PLUGIN_ID", "dev.panemorph"),
            source_pane_id,
            mode,
        )
        completed = subprocess.run(command, check=False, capture_output=True, text=True)
        if completed.returncode:
            message = completed.stderr.strip() or completed.stdout.strip() or "selector failed"
            raise HerdrError(message)
    except HerdrError as error:
        try:
            client.notify("paneMorph selector could not open", str(error))
        except UnboundLocalError:
            pass
        print(f"paneMorph: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
