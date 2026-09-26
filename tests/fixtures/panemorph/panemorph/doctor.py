from __future__ import annotations

import sys

from .api import HerdrClient, HerdrError


def main() -> int:
    try:
        client = HerdrClient()
        pong = client.request("ping")
        snapshot = client.snapshot()
    except HerdrError as error:
        print(f"paneMorph doctor: failed: {error}", file=sys.stderr)
        return 1

    print("paneMorph doctor: ok")
    print(f"Herdr response: {pong.get('type', 'unknown')}")
    print(f"Herdr version: {snapshot.get('version', 'unknown')}")
    print(f"Workspaces: {len(snapshot.get('workspaces', []))}")
    print(f"Tabs: {len(snapshot.get('tabs', []))}")
    print(f"Panes: {len(snapshot.get('panes', []))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

