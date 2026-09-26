from __future__ import annotations

import sys

from panemorph.api import HerdrClient, HerdrError, caller_pane_id
from panemorph.service import PaneMorphService


def main() -> int:
    try:
        client = HerdrClient()
        service = PaneMorphService(client)
        pane = service.current(caller_pane_id())
        service.extract(pane)
    except HerdrError as error:
        try:
            client.notify("paneMorph could not extract the pane", str(error))
        except UnboundLocalError:
            pass
        print(f"paneMorph: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

