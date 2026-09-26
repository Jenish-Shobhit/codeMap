from __future__ import annotations

from typing import Any, Iterator

from .api import HerdrError
from .model import MoveStep


def node_type(node: dict[str, Any]) -> str:
    value = node.get("type")
    if value not in {"pane", "split"}:
        raise HerdrError("Herdr returned an invalid layout node")
    return value


def first_leaf(node: dict[str, Any]) -> str:
    if node_type(node) == "pane":
        pane_id = node.get("pane_id")
        if not isinstance(pane_id, str) or not pane_id:
            raise HerdrError("A live layout pane is missing its pane ID")
        return pane_id
    first = node.get("first")
    if not isinstance(first, dict):
        raise HerdrError("A split layout is missing its first child")
    return first_leaf(first)


def leaf_ids(node: dict[str, Any]) -> Iterator[str]:
    if node_type(node) == "pane":
        yield first_leaf(node)
        return
    first = node.get("first")
    second = node.get("second")
    if not isinstance(first, dict) or not isinstance(second, dict):
        raise HerdrError("A split layout is missing a child")
    yield from leaf_ids(first)
    yield from leaf_ids(second)


def _expansion_steps(node: dict[str, Any], destination_tab_id: str) -> list[MoveStep]:
    if node_type(node) == "pane":
        return []
    first = node.get("first")
    second = node.get("second")
    direction = node.get("direction")
    ratio = node.get("ratio", 0.5)
    if not isinstance(first, dict) or not isinstance(second, dict):
        raise HerdrError("A split layout is missing a child")
    if direction not in {"right", "down"}:
        raise HerdrError("A split layout has an unsupported direction")
    if not isinstance(ratio, (int, float)) or not 0 < float(ratio) < 1:
        raise HerdrError("A split layout has an invalid ratio")

    step = MoveStep(
        pane_id=first_leaf(second),
        tab_id=destination_tab_id,
        target_pane_id=first_leaf(first),
        split=direction,
        ratio=float(ratio),
    )
    return [step, *_expansion_steps(first, destination_tab_id), *_expansion_steps(second, destination_tab_id)]


def plan_tab_merge(
    source_root: dict[str, Any],
    destination_tab_id: str,
    destination_pane_id: str,
    *,
    outer_ratio: float = 0.5,
) -> list[MoveStep]:
    """Plan live pane moves that recreate a source BSP tree on the right.

    The first source leaf becomes the placeholder for the imported subtree.
    Each following move expands one placeholder leaf using the source node's
    direction and ratio, so the live PTYs survive while the tree is rebuilt.
    """

    if not 0 < outer_ratio < 1:
        raise ValueError("outer_ratio must be between zero and one")
    root_leaf = first_leaf(source_root)
    initial = MoveStep(
        pane_id=root_leaf,
        tab_id=destination_tab_id,
        target_pane_id=destination_pane_id,
        split="right",
        ratio=outer_ratio,
    )
    return [initial, *_expansion_steps(source_root, destination_tab_id)]

