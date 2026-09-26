from __future__ import annotations

import curses
import json
import os
import sys
from dataclasses import asdict
from pathlib import Path

from panemorph.api import HerdrClient, HerdrError, caller_pane_id
from panemorph.model import Choice
from panemorph.service import PaneMorphService


def clipped(text: str, width: int) -> str:
    if width <= 0:
        return ""
    if len(text) <= width:
        return text
    return text[: max(0, width - 1)] + "…"


def pick(screen: curses.window, choices: list[Choice], mode: str) -> Choice | None:
    try:
        curses.curs_set(1)
    except curses.error:
        pass
    screen.keypad(True)
    query = ""
    selected = 0
    title = "Send current pane to…" if mode == "send" else "Bring into this tab on the right…"

    while True:
        height, width = screen.getmaxyx()
        terms = query.lower().split()
        filtered = [choice for choice in choices if all(term in choice.searchable for term in terms)]
        selected = min(selected, max(0, len(filtered) - 1))

        screen.erase()
        screen.addnstr(0, 0, clipped(f" paneMorph · {title} ", width - 1), width - 1, curses.A_BOLD)
        screen.addnstr(1, 0, clipped("Type to filter · ↑↓ select · Enter move · Esc cancel", width - 1), width - 1, curses.A_DIM)
        screen.addnstr(3, 0, clipped(f"> {query}", width - 1), width - 1)

        top = 5
        visible = max(1, height - top - 1)
        start = max(0, selected - visible + 1)
        if not filtered:
            screen.addnstr(top, 0, clipped("No matching destinations", width - 1), width - 1, curses.A_DIM)
        for row, choice in enumerate(filtered[start : start + visible], start=top):
            index = start + row - top
            prefix = "  " * choice.depth + ("› " if choice.depth else "▸ ")
            suffix = f"  {choice.subtitle}" if choice.subtitle else ""
            style = curses.A_REVERSE if index == selected else curses.A_NORMAL
            screen.addnstr(row, 0, clipped(prefix + choice.title + suffix, width - 1), width - 1, style)

        screen.move(3, min(width - 1, 2 + len(query)))
        screen.refresh()
        key = screen.get_wch()
        if key in ("\x1b", "\x03"):
            return None
        if key in ("\n", "\r", curses.KEY_ENTER):
            return filtered[selected] if filtered else None
        if key in (curses.KEY_UP, "\x10"):
            selected = max(0, selected - 1)
        elif key in (curses.KEY_DOWN, "\x0e"):
            selected = min(max(0, len(filtered) - 1), selected + 1)
        elif key in (curses.KEY_BACKSPACE, "\x7f", "\b"):
            query = query[:-1]
            selected = 0
        elif isinstance(key, str) and key.isprintable():
            query += key
            selected = 0


def main() -> int:
    mode = os.environ.get("PANEMORPH_MODE", "")
    if mode not in {"send", "bring"}:
        print("paneMorph: selector mode is missing", file=sys.stderr)
        return 2
    result_path = os.environ.get("PANEMORPH_RESULT_PATH")
    if not result_path:
        print("paneMorph: selector result path is missing", file=sys.stderr)
        return 2
    try:
        client = HerdrClient()
        service = PaneMorphService(client)
        pane = service.current(caller_pane_id())
        snapshot = service.snapshot_for(pane)
        choices = service.send_choices(pane, snapshot) if mode == "send" else service.bring_choices(pane, snapshot)
        choice = curses.wrapper(pick, choices, mode)
        # Only choose here. The action runner applies the move after Herdr
        # removes this overlay and restores the original tab's zoom state.
        Path(result_path).write_text(json.dumps({"choice": asdict(choice) if choice else None}))
    except (HerdrError, curses.error) as error:
        Path(result_path).write_text(json.dumps({"error": str(error)}))
        print(f"paneMorph: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
