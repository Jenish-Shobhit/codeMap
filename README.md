# codeMap

See your code as a map, and see what your agents changed on it.

codeMap is a [herdr](https://herdr.dev) plugin, a sibling of paneMorph.
Press ⌃⌥M over the agent you are watching: a popup opens on that agent's
worktree with four views. The number keys move between them.

| Key | View | What it shows |
| --- | --- | --- |
| `1` | **Map** | Folders, files and functions as boxes; calls and imports as lines, in a layered layout. What the agent's last turn changed is marked `M` (changed) or `A` (added). |
| `2` | **Flow** | The function under the cursor as a flowchart: decisions, loops, try/except, match, calls and returns. Changed lines are marked. |
| `3` | **Changes** | The hunks since the agent's last turn (or since HEAD), file by file. Mark hunks reviewed, write comments, send them to the agent's prompt. |
| `4` | **History** | The git graph (lanes, branches, tags, HEAD) and the worktrees with the agents working in them. Enter shows a commit's diff. |

It also runs on its own, outside herdr: `codemap [path]`.

```text
 map                                      panemorph/  1 folder · 6 modules · 39 functions · 3 changed this turn
 paneMorph/        11 files
   panemorph/      10 files                                        ╭─ actions/ ────────────╮
                                                                   │ 4 files · 7 functions │
 this turn · 4 functions                                           │ 2 changed           M │
 M selector_command   open_selector                                ╰─────┬─────┬─────┬─────╯
 A finish_selection   open_selector                                      │     │     │
 M main               open_selector                      ┌───────────────┘     │     └───────────────┐
 M main               selector                           │                     ▼ PaneMorphService +6 │
                                                         │    ┌─ service.py · PaneMorphService ─┐    │
                                                         │    │ __init__                        │    │
                                                         │    │ _id                             │    │
```

```text
                          ┌─────────────────────────────────┐           ╭──────────────────────────────────╮
                          │ ◇ mode not in {"send", "bring"} ├─── yes ──▶│ print("paneMorph: selector mode… │
                          └────────────────┬────────────────┘           │ ◉ return 2                       │
                                           │ no                         ╰──────────────────────────────────╯
                                           ▼
               ┌───────────────────────────────────────────────────────┐
       A       │ result_path = os.environ.get("PANEMORPH_RESULT_PATH") │
               └───────────────────────────┬───────────────────────────┘
```

These frames are real: they are snapshot tests (`tests/snapshots/`) rendered
from paneMorph's code, with its commit b8b645e standing in for an agent turn.

## Build

codeMap is a Rust program (edition 2021, Rust 1.85 or newer). tree-sitter's
grammars are C, so a C compiler is needed too (Apple clang on macOS).

```sh
cd ~/Desktop/codeMap
cargo build --release
```

The binary is `target/release/codemap`. The plugin manifest runs it from
there: herdr resolves a relative command with a slash against the plugin's
own directory.

## Install in herdr

Linking registers the plugin for your user, in every herdr session. Run these
yourself:

```sh
cargo build --release                  # linking does not build
herdr plugin link ~/Desktop/codeMap
herdr plugin list                      # dev.codemap, enabled, no warnings
herdr plugin action list --plugin dev.codemap
```

Then bind ⌃⌥M. Add this to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "ctrl+alt+m"
type = "plugin_action"
command = "dev.codemap.open"
description = "codeMap: map of the focused agent's code"
```

and check and reload the config:

```sh
herdr config check
herdr server reload-config
```

Optional bindings for the other views use the same shape with
`dev.codemap.flow`, `dev.codemap.changes` and `dev.codemap.history`.

⌃⌥M needs a terminal that reports it as itself. Without the Kitty keyboard
protocol, ⌃M is the same byte as Return, so ⌃⌥M can arrive as ⌥⏎. Ghostty
supports the protocol. If the chord does nothing, try it in Ghostty, or bind
another free chord such as `prefix+m`.

To remove it: `herdr plugin unlink dev.codemap`.

## Using it

- **Inside herdr**, ⌃⌥M opens a popup at 94% × 92% over the focused pane.
  codeMap opens on that pane's working directory (its foreground cwd, so it
  follows an agent that changed directory, then its herdr worktree). The
  header shows the agent, the branch and the turn: `○ paneMorph ·
  selector-fix   main   turn 3 · 2m`. The popup takes every key, so nothing
  reaches herdr or the agent until it closes.
- **Standalone**, `codemap [path]` shows the same views. Without an agent,
  Changes starts at "since HEAD"; turns recorded by the hook in that
  repository are still available with `s`.

### Keys

| Keys | Where | Does |
| --- | --- | --- |
| `1 2 3 4` | everywhere | Map, Flow, Changes, History |
| `tab` | Map, Flow, Changes | move between the rail and the body |
| `/` | everywhere | search a function, class or file; `↑ ↓` pick, `⏎` go |
| `e`, `y` | Map, Flow, Changes | open `$EDITOR` at the line; copy `path:line` |
| `p`, `T` | inside herdr | pin codeMap as a split that follows the focused pane; open it in its own tab |
| `?`, `q` or `⎋` | everywhere | show all keys; close |
| `← → ↑ ↓` | Map | move between boxes |
| `j k` | Map | move through a box's rows (a file's functions, a class's methods) |
| `⏎`, `⌫` | Map | zoom in (folder → file → Flow of a function), zoom out |
| `] [` | Map | next or previous page in a big folder |
| `↑ ↓ ← →`, `⏎`, `⌫` | Flow | move between boxes, open the call in the box, go back |
| `j k`, `] [`, `} {` | Changes | line, hunk, file |
| `space` | Changes | mark the hunk reviewed (kept until the hunk changes) |
| `v`, `c`, `x` | Changes | select lines, comment on them, delete a comment |
| `P`, `S` | Changes | type the draft into the agent's prompt and close (you press Enter); submit it at once with `agent.prompt` |
| `s`, `a` | Changes, Map | scope: last turn → this agent's session → since HEAD; next agent |
| `j k`, `⏎`, `⌫` | History | move between commits, show the diff, back |
| `1` | History | the map with this commit's changes marked |

Comments go to the agent as one message:

```text
Review comments from codeMap:

panemorph/actions/open_selector.py:43-45 — this loop has no timeout; stop after a few seconds
```

`P` wraps multi-line drafts in bracketed paste so the agent receives them as
one paste. `S` uses herdr's `agent.prompt`; if the agent is waiting on a
question, codeMap says so and keeps the draft, and `P` pastes instead.

## How turns are tracked

The manifest's `[[events]]` hook runs `codemap hook` on
`pane.agent_status_changed`. A change to `working` starts a turn; a change from
`working` to `idle`, `done` or `blocked` ends it. Each boundary snapshots the
agent's worktree. Answering a question (`blocked` → `working`) continues the
same turn. Panes are keyed by herdr's terminal id and the agent name, because
pane ids change when panes move.

**codeMap never writes into your repository.** No refs, no stash, no index
changes. Snapshots live in a shadow git dir in the plugin's state directory
(`~/.local/state/herdr/plugins/dev.codemap/repos/<repo>/shadow.git`), with
its own objects, index and refs, and the worktree as its work tree. Commands
that write objects never see your object store (git would otherwise "freshen",
that is touch, objects it finds there); read-only diffs borrow it per process
through `GIT_ALTERNATE_OBJECT_DIRECTORIES`. The "since HEAD" diff reads a
private copy of your index. The test suite fingerprints every file in `.git`
before and after, and the hook runs about 60 ms per turn boundary.

The last 50 turns per agent are kept. Reviewed marks and unsent comments are
stored next to them.

## States

| When | codeMap shows |
| --- | --- |
| Loading | The frame, tabs and rail first; boxes fill in as files are parsed. |
| Not a git repo | Map and Flow still work. Changes and History say `not a git repository`. |
| Empty repo | Changes lists every file as added; History says `no commits yet`. |
| No turn yet | `no turn checkpoint yet · showing since HEAD`. |
| Huge repo | The map opens at folder level and parses a folder when you zoom into it; big folders page with `] [`. Listing is capped at 20,000 files. |
| Unsupported language | The file is a plain box with its size. Flow says which grammars exist. |
| Binary file | `binary · 24 KB`, no content. |

Languages with Map and Flow: Python, Rust, TypeScript/TSX, JavaScript and Go.

## Theme

herdr has no theme API, so codeMap reads `[theme]` from herdr's
`config.toml` (`name`, `auto_switch`/`dark_name`, `[theme.custom]` and
`[theme.custom.dark]`) and resolves it the way herdr does, with herdr's 18
built-in palettes ported from its source. With dracula and black custom
surfaces: rail `#000000`, body `#282a36`, selection `#1a1a1a`, active row
`#141414`, accent `#bd93f9` on the active view label only, branches `#ff79c6`,
added/removed bands at 15% green/red over the body.

## Performance

Measured on an M-series Mac, release build:

| What | Time |
| --- | --- |
| Process start to first paint (frame, tabs, rail) | 0.6 ms in process; first byte on the pty 2.6 ms, full frame 5.1 ms after spawn |
| Small repo (paneMorph, 11 files): content loaded | 66–100 ms |
| codeMap itself (63 files, Rust): content loaded | ~230 ms |
| 12,000-file repo: first paint / loaded | 0.3 ms / ~110 ms (parses a folder on zoom, ~7 ms) |
| History: 2,000 commits, lanes and frame | ~20 ms |
| Hook, per turn boundary | ~60 ms |

The first launch after every build is slower (about 0.7 s) while macOS checks
the new binary once.

## Troubleshooting

- `codemap doctor` prints where codeMap looks: herdr socket, config,
  theme, state dir, and how many agents have turns in the current repo.
- `herdr plugin log list --plugin dev.codemap` shows the action and hook
  runs, with their output.
- Nothing is marked `M`/`A`: no turn was recorded yet (the hook runs once the
  plugin is linked), so the scope is "since HEAD". Press `s` to cycle.

## Development

```sh
cargo test                   # everything below except the live tests
cargo clippy --all-targets
cargo fmt
```

- `tests/index.rs`: tree-sitter extraction and call-edge resolution on
  paneMorph's real Python (copied into `tests/fixtures/`), Rust, TypeScript
  and Go.
- `tests/layout.rs`: 200 random graphs, no overlapping boxes, no edge through
  a box.
- `tests/flow.rs`: if/elif/else, loops, try/except, match and returns per
  language; every paneMorph function charts without overlaps.
- `tests/views.rs`: ratatui `TestBackend` snapshots of all four views and the
  states, colours, and keys. Regenerate with `UPDATE_SNAPSHOTS=1`.
- `tests/git_store.rs`, `tests/hook.rs`: turn diffs from checkpoints, the
  shadow store writing nothing into the repo, graph, worktrees, the hook.
- `tests/manifest.rs`: `herdr-plugin.toml` against herdr 0.9.0's link rules.
- `tests/pty.rs`: the real binary on a pseudo-terminal: keys, Esc closing,
  first-paint time. `CODEMAP_BIN=target/release/codemap` measures a
  release build.
- `tests/perf.rs` (ignored): 2,000 commits, 12,000 files.
- `tests/live_herdr.rs` (ignored): against a throwaway herdr server, never
  your own session. See the file header for how to start one with
  `XDG_CONFIG_HOME`/`XDG_STATE_HOME` pointing at a temp dir.

## License

MIT, see [LICENSE](LICENSE). The colour palettes are ported from herdr
(Apache-2.0), see [NOTICE](NOTICE).
