# Changelog

All notable changes to codeMap are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Files**: a popup with the worktree as a tree and the selected file's
  contents beside it, opened with `dev.codemap.files` or `--view files`.
  Files the turn touched are marked, and added lines carry a `+`. `/` finds
  a file by name.
- **Customisations**: a popup that lists what the agent runs with, opened
  with `dev.codemap.customisations` or `--view customisations`. Four scopes:
  the project's `CLAUDE.md`, settings, skills, agents, commands, hooks, MCP
  servers and memory; the same from `~/.claude`, with plugins; the
  configuration of herdr, yazi, neovim and other tools; and other agents'
  files. Secrets in those files are never shown.
- **Status**: the first tab of the Git popup, opened with `dev.codemap.git`
  or `--view status`. It lists the staged and the not staged files, untracked
  ones with a `?`, and shows the selected file's diff: the index against
  `HEAD` for a staged file, the working tree against the index for the
  others.
- **Staging, committing and discarding** from Status. `space` stages or
  unstages the selected file, or in the diff the hunk under the cursor, which
  `]` and `[` move and a bar marks. `c` opens a commit box with a subject and
  a body; a commit git refuses keeps the box and the text. `x` restores a
  file or a hunk from the index, or deletes an untracked file, after a
  question that only `y` confirms.
- `E` opens the file in VS Code, `b` in the browser and `B` on GitHub, from
  Files, Customisations, Status and Review.

### Changed

- codeMap now writes into the repository, from three keys in the Status tab
  and nowhere else: `space` the index, `c` a commit, `x` the working tree.
  Opening, reading and reviewing still write nothing.
- `e` opens the editor in a herdr split beside the agent's pane and closes
  the popup. Outside herdr it still takes over the terminal. With `$VISUAL`
  and `$EDITOR` unset the editor is `nvim` when installed, where it was `vi`.
- Each popup shows only its own views: `1`, `2` and `3` switch between
  Status, Review and History, and Files stands alone.
- The Changes tab is labelled `review`, beside `status` and `history`.
  `--view changes`, `--view review` and the `dev.codemap.changes` action open
  it as before.

- `dev.codemap.open` opens Review, where it opened the Map.
- Hunks in Review are named by the function line git prints in the hunk
  header, no longer by a parsed symbol.
- Building from source no longer compiles C: the binary has no tree-sitter
  runtime or grammars.
- Upgraded `toml` from 0.8 to 1.1. herdr's `[theme]` settings resolve as
  before; `config.toml` files that use TOML 1.1 syntax, such as multi-line
  inline tables, are now read instead of falling back to the default theme.

### Removed

- The Map and Flow views, and with them symbol search (`/`), the
  `dev.codemap.flow` action, `--view map` and `--view flow`, and the `1` key
  in History that opened the Map on a commit. Bind `dev.codemap.open` or
  `dev.codemap.changes` in place of `dev.codemap.flow`.

## [0.1.0] - 2026-09-27

First public release. codeMap was developed under the working names Loupe
and codeMorph.

### Added

- One herdr popup (94% × 92%) over the focused agent's worktree, opened
  with a key bound to `dev.codemap.open`, with four views switched by
  `1`–`4`. The same views run outside herdr as `codemap [path]`.
- **Map**: folders, files, classes and functions as boxes, calls and imports
  as lines, in a layered (Sugiyama) layout drawn with box characters. Zoom
  with `⏎` and `⌫`, search with `/`, move with the arrow keys. Functions the
  agent's last turn changed are marked `M`, added ones `A`.
- **Flow**: flowcharts of Python, Rust, TypeScript/JavaScript and Go
  functions (decisions, loops, try/except, match, returns), with changed
  lines marked. `⏎` opens a call, `⌫` goes back.
- **Changes**: hunks since the last turn, the agent's session or `HEAD`.
  Reviewed marks persist until a hunk changes; line comments go to the
  agent's prompt with `P` (paste) or `S` (herdr's `agent.prompt`).
- **History**: the commit graph with branches, tags and `HEAD`, the
  worktrees with the agents working in them, and commit diffs.
- Turn checkpoints from the `pane.agent_status_changed` hook, stored in a
  shadow git directory in the plugin's state directory. Nothing is written
  into the repository.
- herdr's theme: all 18 built-in palettes and `[theme.custom]` overrides.
- States for loading, not a repository, an empty repository, huge
  repositories (lazy parsing, paged folders) and unsupported languages.
- `codemap doctor`, which prints where codeMap looks for herdr, its theme
  and its state.
- Prebuilt binaries for macOS and Linux on x86_64 and aarch64.

[Unreleased]: https://github.com/Jenish-Shobhit/codeMap/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Jenish-Shobhit/codeMap/releases/tag/v0.1.0
