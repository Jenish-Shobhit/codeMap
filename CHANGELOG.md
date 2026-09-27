# Changelog

All notable changes to codeMap are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Upgraded `tree-sitter` from 0.25 to 0.27. The grammar crates stay on
  their latest releases (`tree-sitter-python` 0.25, `tree-sitter-rust` 0.24,
  `tree-sitter-typescript` 0.23, `tree-sitter-javascript` 0.25,
  `tree-sitter-go` 0.25), whose ABIs (14 and 15) tree-sitter 0.27 supports.
  Symbols, calls, imports and flowcharts are unchanged.
- Upgraded `toml` from 0.8 to 1.1. herdr's `[theme]` settings resolve as
  before; `config.toml` files that use TOML 1.1 syntax, such as multi-line
  inline tables, are now read instead of falling back to the default theme.

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
