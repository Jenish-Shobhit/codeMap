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
- **Branches**: the fourth tab of the Git popup, on `4` or `--view branches`.
  It lists the local branches with how far each is ahead of and behind its
  upstream, and one line per remote. The selected branch shows its upstream,
  the worktree it is checked out in, and the commits it has that the default
  branch does not.
- **Switching, creating, merging and deleting branches** from Branches. `⏎`
  runs `git switch`, and asks first when the agent in the worktree is
  working. `n` takes a name and runs `git switch -c`. `m` runs `git merge
  --no-edit` after a question, and leaves a merge that stops on conflicts for
  the Status tab. `D` runs `git branch -D` after a question that counts the
  commits no other branch has; the current branch and one checked out in
  another worktree are refused.
- **Pull requests and CI** in Branches, read with your own `gh`: each
  branch's pull request number with `✓`, `✗` or `…` for its checks, and its
  title, state and base branch. `B` opens the pull request, or the branch on
  GitHub. Without `gh` the tab says why and shows the rest. `CODEMAP_GH`
  names another program.
- **Worktrees**: the fifth tab of the Git popup, on `5` or `--view
  worktrees`. It lists every worktree of the repository with its branch, the
  herdr agents working in it and a summary of its uncommitted changes, and
  marks the one the popup is showing. The selected worktree shows its
  upstream, each agent and its state, and its staged and not staged files; a
  locked worktree and one whose folder is missing say so. The tab only reads,
  through a private copy of each worktree's index.
- **Jumping between worktrees**: `⏎` in Worktrees points the whole popup at
  the selected worktree, so Status, Review, History and Branches show it, and
  follows the agent herdr reports working there. `y` copies the worktree's
  path, and `e` and `E` open its folder in the editor and in VS Code.
- **History of one file**: `L` on a file in Files, Status, Review, or on a
  file inside the repository in Customisations, lists only the commits that
  touched it, following renames, as a plain list without the graph. `⏎`
  shows what the selected commit did to that file alone, under the name it
  had then. `⎋` goes back to where `L` was pressed; in the Git popup `3`
  still shows the full history. `e`, `E` and `b` open the file.
- **Commits on GitHub**: `B` in History opens the selected commit on GitHub
  when a branch on `origin` already holds it, and otherwise says why not;
  `y` copies its full sha. Both read only the local repository.

### Changed

- `?` lists the keys of the popup that is open, no longer every popup's,
  and `j` `k` scroll the list when it is taller than the window.
- A renamed file in a diff says the name it was renamed from.

- codeMap now writes into the repository, from three keys in the Status tab
  and four in the Branches tab, and nowhere else: `space` the index, `c` a
  commit, `x` the working tree, `⏎` and `n` the checked-out branch, `m` a
  merge and `D` a local branch. Opening, reading and reviewing still write
  nothing.
- codeMap now runs a program that uses the network: the Branches tab runs
  `gh pr list` once, when it is first shown and `gh` is installed, and `gh`
  contacts GitHub with your login. codeMap itself still opens no network
  connection.
- `e` opens the editor in a herdr split beside the agent's pane and closes
  the popup. Outside herdr it still takes over the terminal. With `$VISUAL`
  and `$EDITOR` unset the editor is `nvim` when installed, where it was `vi`.
- Each popup shows only its own views: `1` to `5` switch between Status,
  Review, History, Branches and Worktrees, and Files stands alone.
- History no longer lists the worktrees under the graph, now that they have a
  tab of their own. Its header still counts them.
- The Changes tab is labelled `review`, beside `status`, `history` and
  `branches`. `--view changes`, `--view review` and the `dev.codemap.changes`
  action open it as before.

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

### Fixed

- Two reads of one worktree's index at the same moment, from two popups or
  from two tabs of one, no longer disturb each other. Each read now copies
  the index under a name of its own; they shared one file, and a read could
  find it missing and show every file as deleted or untracked. Copies left by
  a popup that was closed in the middle of a read are removed by the next
  one.

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
