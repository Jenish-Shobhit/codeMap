from __future__ import annotations

from typing import Any

from .api import HerdrClient, HerdrError
from .model import Choice, MoveStep
from .topology import leaf_ids, plan_tab_merge


class PaneMorphService:
    def __init__(self, client: HerdrClient) -> None:
        self.client = client

    @staticmethod
    def _id(record: dict[str, Any], key: str) -> str:
        value = record.get(key)
        return value if isinstance(value, str) else ""

    @staticmethod
    def _layout(snapshot: dict[str, Any], tab_id: str) -> dict[str, Any]:
        for layout in snapshot.get("layouts", []):
            if isinstance(layout, dict) and layout.get("tab_id") == tab_id:
                return layout
        raise HerdrError(f"Layout for {tab_id} is unavailable")

    @staticmethod
    def _tab(snapshot: dict[str, Any], tab_id: str) -> dict[str, Any]:
        for tab in snapshot.get("tabs", []):
            if isinstance(tab, dict) and tab.get("tab_id") == tab_id:
                return tab
        raise HerdrError(f"Tab {tab_id} is unavailable")

    @staticmethod
    def _pane_title(pane: dict[str, Any]) -> str:
        for key in ("terminal_title_stripped", "terminal_title", "agent", "pane_id"):
            value = pane.get(key)
            if isinstance(value, str) and value.strip():
                return value.strip()
        return "Unnamed pane"

    @staticmethod
    def _tab_title(tab: dict[str, Any]) -> str:
        label = tab.get("label")
        if isinstance(label, str) and label.strip():
            return label.strip()
        number = tab.get("number")
        return f"Tab {number}" if number is not None else "Unnamed tab"

    def current(self, caller_pane_id: str) -> dict[str, Any]:
        return self.client.current_pane(caller_pane_id)

    def snapshot_for(self, pane: dict[str, Any]) -> dict[str, Any]:
        snapshot = self.client.snapshot()
        pane_id = self._id(pane, "pane_id")
        if not any(
            isinstance(item, dict) and item.get("pane_id") == pane_id
            for item in snapshot.get("panes", [])
        ):
            raise HerdrError("The source pane changed before paneMorph could act")
        return snapshot

    def _ensure_unzoomed(self, snapshot: dict[str, Any], *tab_ids: str) -> None:
        for tab_id in tab_ids:
            if self._layout(snapshot, tab_id).get("zoomed") is True:
                raise HerdrError("Unzoom the source and destination tabs, then try again")

    def extract(self, pane: dict[str, Any]) -> dict[str, Any]:
        snapshot = self.snapshot_for(pane)
        tab_id = self._id(pane, "tab_id")
        workspace_id = self._id(pane, "workspace_id")
        self._ensure_unzoomed(snapshot, tab_id)
        return self.client.request(
            "pane.move",
            {
                "pane_id": self._id(pane, "pane_id"),
                "destination": {
                    "type": "new_tab",
                    "workspace_id": workspace_id,
                },
                "focus": False,
            },
        )

    def send_choices(self, pane: dict[str, Any], snapshot: dict[str, Any]) -> list[Choice]:
        current_tab = self._id(pane, "tab_id")
        workspace_id = self._id(pane, "workspace_id")
        choices = [Choice("new_tab", "New background tab", "Create after the current tab")]
        tabs = [
            tab
            for tab in snapshot.get("tabs", [])
            if isinstance(tab, dict)
            and tab.get("workspace_id") == workspace_id
            and tab.get("tab_id") != current_tab
        ]
        tabs.sort(key=lambda tab: int(tab.get("number", 0)))
        for tab in tabs:
            count = int(tab.get("pane_count", 0))
            choices.append(
                Choice(
                    "tab",
                    self._tab_title(tab),
                    f"{count} pane{'s' if count != 1 else ''}",
                    tab_id=self._id(tab, "tab_id"),
                )
            )
        return choices

    def bring_choices(self, pane: dict[str, Any], snapshot: dict[str, Any]) -> list[Choice]:
        current_tab = self._id(pane, "tab_id")
        workspace_id = self._id(pane, "workspace_id")
        panes_by_tab: dict[str, list[dict[str, Any]]] = {}
        for candidate in snapshot.get("panes", []):
            if isinstance(candidate, dict) and candidate.get("workspace_id") == workspace_id:
                panes_by_tab.setdefault(self._id(candidate, "tab_id"), []).append(candidate)

        tabs = [
            tab
            for tab in snapshot.get("tabs", [])
            if isinstance(tab, dict)
            and tab.get("workspace_id") == workspace_id
            and tab.get("tab_id") != current_tab
        ]
        tabs.sort(key=lambda tab: int(tab.get("number", 0)))
        choices: list[Choice] = []
        for tab in tabs:
            tab_id = self._id(tab, "tab_id")
            children = panes_by_tab.get(tab_id, [])
            choices.append(
                Choice(
                    "tab",
                    self._tab_title(tab),
                    f"Move entire layout · {len(children)} pane{'s' if len(children) != 1 else ''}",
                    tab_id=tab_id,
                )
            )
            children.sort(key=lambda item: self._pane_title(item).lower())
            for child in children:
                cwd = child.get("foreground_cwd") or child.get("cwd") or ""
                choices.append(
                    Choice(
                        "pane",
                        self._pane_title(child),
                        str(cwd),
                        tab_id=tab_id,
                        pane_id=self._id(child, "pane_id"),
                        depth=1,
                    )
                )
        return choices

    def _target_pane(self, snapshot: dict[str, Any], tab_id: str) -> str:
        layout = self._layout(snapshot, tab_id)
        focused = layout.get("focused_pane_id")
        if isinstance(focused, str) and focused:
            return focused
        panes = layout.get("panes", [])
        if panes and isinstance(panes[0], dict):
            pane_id = panes[0].get("pane_id")
            if isinstance(pane_id, str) and pane_id:
                return pane_id
        raise HerdrError("The destination tab has no pane")

    @staticmethod
    def _move_result(result: dict[str, Any]) -> dict[str, Any]:
        move_result = result.get("move_result")
        if not isinstance(move_result, dict):
            raise HerdrError("Herdr did not return a pane move result")
        if move_result.get("changed") is False:
            reason = move_result.get("reason") or "unchanged"
            raise HerdrError(f"Herdr did not move the pane: {reason}")
        return move_result

    def send(self, pane: dict[str, Any], choice: Choice) -> None:
        snapshot = self.snapshot_for(pane)
        source_tab = self._id(pane, "tab_id")
        self._ensure_unzoomed(snapshot, source_tab)
        if choice.kind == "new_tab":
            self._move_result(
                self.client.request(
                    "pane.move",
                    {
                        "pane_id": self._id(pane, "pane_id"),
                        "destination": {
                            "type": "new_tab",
                            "workspace_id": self._id(pane, "workspace_id"),
                        },
                        "focus": False,
                    },
                )
            )
            return
        if choice.kind != "tab" or not choice.tab_id:
            raise HerdrError("Choose a destination tab")
        self._ensure_unzoomed(snapshot, choice.tab_id)
        target_pane = self._target_pane(snapshot, choice.tab_id)
        self._move_result(
            self.client.request(
                "pane.move",
                MoveStep(
                    pane_id=self._id(pane, "pane_id"),
                    tab_id=choice.tab_id,
                    target_pane_id=target_pane,
                    split="right",
                ).params(),
            )
        )

    def bring(self, destination_pane: dict[str, Any], choice: Choice) -> None:
        snapshot = self.snapshot_for(destination_pane)
        destination_tab = self._id(destination_pane, "tab_id")
        self._ensure_unzoomed(snapshot, destination_tab)
        if choice.kind == "pane" and choice.pane_id and choice.tab_id:
            self._ensure_unzoomed(snapshot, choice.tab_id)
            self._move_result(
                self.client.request(
                    "pane.move",
                    MoveStep(
                        pane_id=choice.pane_id,
                        tab_id=destination_tab,
                        target_pane_id=self._id(destination_pane, "pane_id"),
                        split="right",
                    ).params(),
                )
            )
            return
        if choice.kind != "tab" or not choice.tab_id:
            raise HerdrError("Choose a source tab or pane")
        self._ensure_unzoomed(snapshot, choice.tab_id)
        export = self.client.request("layout.export", {"tab_id": choice.tab_id})
        layout = export.get("layout")
        if not isinstance(layout, dict) or not isinstance(layout.get("root"), dict):
            raise HerdrError("Herdr could not export the selected tab layout")
        plan = plan_tab_merge(
            layout["root"],
            destination_tab,
            self._id(destination_pane, "pane_id"),
        )
        self._execute_merge(plan, choice.tab_id, layout["root"])

    def _execute_merge(
        self,
        plan: list[MoveStep],
        source_tab_id: str,
        source_root: dict[str, Any],
    ) -> None:
        moved: list[str] = []
        all_panes = list(leaf_ids(source_root))
        try:
            for step in plan:
                self._move_result(self.client.request("pane.move", step.params()))
                moved.append(step.pane_id)
        except HerdrError as error:
            remaining = [pane_id for pane_id in all_panes if pane_id not in moved]
            rollback_error = self._rollback(source_tab_id, moved, remaining)
            if rollback_error:
                raise HerdrError(
                    f"{error}. Rollback also failed: {rollback_error}. "
                    "All terminal processes are still running; inspect the affected tabs."
                ) from error
            raise HerdrError(f"{error}. The partial move was rolled back.") from error

    def _rollback(self, source_tab_id: str, moved: list[str], remaining: list[str]) -> str:
        if not moved:
            return ""
        if not remaining:
            return "source tab closed before rollback"
        anchor = remaining[0]
        try:
            for pane_id in reversed(moved):
                self._move_result(
                    self.client.request(
                        "pane.move",
                        MoveStep(
                            pane_id=pane_id,
                            tab_id=source_tab_id,
                            target_pane_id=anchor,
                            split="down",
                        ).params(),
                    )
                )
                anchor = pane_id
        except HerdrError as error:
            return str(error)
        return ""

