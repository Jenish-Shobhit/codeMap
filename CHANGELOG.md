# Changelog

All notable changes to codeMorph will be documented here.

## 0.1.0 — 2026-09-27

- One herdr popup (94% × 92%) with four views on the focused agent's
  worktree: Map, Flow, Changes and History, switched with 1–4. Also runs on
  its own as `codemorph [path]`.
- Map: folders, files and functions as boxes, calls and imports as lines in a
  layered (Sugiyama) layout drawn with box characters; zoom with ⏎ and ⌫,
  search with /, move with the arrow keys; functions changed in the agent's
  last turn marked M or A.
- Flow: flowcharts of Python, Rust, TypeScript/JavaScript and Go functions
  (decisions, loops, try/except, match, returns) with changed lines marked;
  ⏎ opens a call, ⌫ goes back.
- Changes: hunks since the last turn, the agent's session or HEAD; reviewed
  marks that persist; comments sent to the agent with P (paste) or S
  (`agent.prompt`).
- History: commit graph with branches, tags and HEAD, worktrees with their
  agents, and commit diffs.
- Turn checkpoints from the `pane.agent_status_changed` hook, stored in a
  shadow git dir in the plugin state directory; nothing is written into the
  repository.
- herdr's theme (all 18 palettes, `[theme.custom]` overrides), states for
  loading, no repo, empty repo, huge repo and unsupported languages.
