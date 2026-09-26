from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal


Direction = Literal["right", "down"]


@dataclass(frozen=True)
class MoveStep:
    pane_id: str
    tab_id: str
    target_pane_id: str
    split: Direction
    ratio: float = 0.5

    def params(self, *, focus: bool = False) -> dict[str, Any]:
        return {
            "pane_id": self.pane_id,
            "destination": {
                "type": "tab",
                "tab_id": self.tab_id,
                "target_pane_id": self.target_pane_id,
                "split": self.split,
                "ratio": self.ratio,
            },
            "focus": focus,
        }


@dataclass(frozen=True)
class Choice:
    kind: Literal["new_tab", "tab", "pane", "disabled"]
    title: str
    subtitle: str = ""
    tab_id: str | None = None
    pane_id: str | None = None
    depth: int = 0

    @property
    def searchable(self) -> str:
        return f"{self.title} {self.subtitle} {self.tab_id or ''} {self.pane_id or ''}".lower()

