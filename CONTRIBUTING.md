# Contributing to codeMap

Thanks for helping. Bug reports, fixes, new language support and documentation
are all welcome. For a larger change, open an issue first so we can agree on
the approach before you write it.

By contributing you agree that your work is licensed under the
[MIT License](LICENSE), and to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Development setup

You need:

- **Rust**: `rust-toolchain.toml` selects stable with rustfmt and clippy, and
  rustup installs it on first use. The minimum supported version is 1.90
  (`rust-version` in `Cargo.toml`).
- **A C compiler**: the tree-sitter grammars are C. Xcode's command line tools
  on macOS; `gcc` or `clang` on Linux.
- **git** on `PATH`: codeMap and its tests run it.
- **herdr 0.9.0 or newer**, only to try the plugin inside herdr.

```sh
git clone https://github.com/Jenish-Shobhit/codeMap
cd codeMap
cargo build
cargo run -- .          # codeMap on its own source, outside herdr
```

`cargo run -- --help` lists the subcommands. `codemap doctor` prints where
codeMap looks for herdr's socket, config, theme and state.

## Checks

CI runs these on Linux and macOS. Run them before you open a pull request:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
```

CI also runs the test suite on Rust 1.90, `cargo deny check` (deny.toml), the
Kani proofs (below), actionlint and zizmor on the workflows, and checks that
the README screenshots are current. A last job, `CI gate`, passes only when
every other job passed; it is the check `main` requires.

## Formal verification

Some promises are proved with [Kani](https://model-checking.github.io/kani/)
for every input within stated bounds, on the real functions. Each proof lives
in a `#[cfg(kani)] mod proofs` at the end of the file it checks, with a header
that states the promise, the bounds and the commit it was read at. A
`.mutations` file beside that file lists broken variants of the code that each
proof must catch, so a proof that can no longer fail does not pass quietly.

```sh
cargo install --locked kani-verifier --version 0.68.0   # once
cargo kani setup                                         # once
python3 scripts/kani.py prove       # every harness holds and reaches every cover
python3 scripts/kani.py mutations   # every broken variant is caught
```

| Proof | Promise |
| --- | --- |
| `ui::proofs::follow_shows_a_box_that_fits` | the Map and Flow views scroll a selected box that fits the view fully on screen |
| `ui::proofs::follow_never_scrolls_past_the_content` | those views never scroll past the canvas |

Each verified area has a project skill in `.claude/skills/` that says which
code it covers, how to rerun it, how to tell a bug from a modelling error, and
how to update it when the promise changes on purpose
(`.claude/skills/selection-scroll/SKILL.md` for the proofs above). If your
change touches covered code, run both commands; if a proof fails, do not
weaken it to pass: the failure is a finding.

## Tests

| File | What it covers |
| --- | --- |
| `tests/index.rs` | tree-sitter extraction and call and import resolution for Python, Rust, TypeScript and Go |
| `tests/layout.rs` | 200 random graphs: no overlapping boxes, no edge through a box |
| `tests/flow.rs` | flowcharts for branches, loops, try/except, match and returns in each language |
| `tests/views.rs` | golden frames of all four views and their states, colours and keys |
| `tests/git_store.rs`, `tests/hook.rs` | turn diffs from checkpoints, the side store writing nothing into the repository, the graph, worktrees, the hook |
| `tests/manifest.rs` | `herdr-plugin.toml` against herdr 0.9.0's link rules |
| `tests/pty.rs` | the real binary on a pseudo-terminal: keys, Esc, first-paint time |

### Snapshot tests

`tests/views.rs` renders each view through ratatui's `TestBackend` and compares
the frame with a golden file in `tests/snapshots/`. When a change to the UI is
intended:

```sh
UPDATE_SNAPSHOTS=1 cargo test --test views
git diff tests/snapshots/
```

Read the diff before you commit it: it is the review of what users will see.
A failing comparison writes `<name>.txt.new` next to the golden file (ignored
by git).

### Screenshots

The images in `docs/assets/` are rendered from the same code:

```sh
cargo run --example screenshots             # rewrites docs/assets/*.svg
cargo run --example screenshots -- --text   # also prints each frame
```

If your change alters a view, regenerate them and commit the result; CI fails
when they are stale.

The README's hero image, `docs/assets/herdr.svg`, is a capture of a real herdr
client, so it needs herdr installed and CI does not check it:

```sh
cargo build --release
cargo run --example screenshots -- --herdr
```

It runs a throwaway herdr session whose `HOME` and XDG directories live in
`/tmp/cmtest-shot-<pid>`, registers codeMap only there, and removes it all
afterwards. It refuses to write an image that shows your user or host name.

### Ignored tests

Some tests are `#[ignore]` because they need a release build, time, or a herdr
server:

```sh
cargo test --release --test perf -- --ignored --nocapture        # 2,000 commits, 12,000 files
CODEMAP_BIN=target/release/codemap \
  cargo test --test pty smoke -- --ignored --nocapture           # codeMap on its own repository
```

### Live tests against herdr

`tests/live_herdr.rs` drives a real herdr server. Run it only against a
throwaway server with its own config and state directories, never your own
session. The test refuses any socket outside a `cmtest-*` directory.

```sh
B=$(mktemp -d /tmp/cmtest-XXXX); mkdir -p "$B/cfg" "$B/state"
env -u HERDR_SOCKET_PATH -u HERDR_ENV \
  XDG_CONFIG_HOME="$B/cfg" XDG_STATE_HOME="$B/state" SHELL=/bin/sh \
  herdr --session cmtest-live server &
CODEMAP_LIVE_SOCKET="$B/cfg/herdr/sessions/cmtest-live/herdr.sock" \
  cargo test --test live_herdr -- --ignored --nocapture
```

Stop the throwaway server when you are done (`kill %1`) and delete `$B`.

## Trying the plugin in herdr

Build first: `herdr plugin link` does not run the manifest's build step.

```sh
cargo build --release
herdr plugin link "$PWD"
```

To keep it out of your everyday setup, link it inside a throwaway herdr whose
`XDG_CONFIG_HOME` and `XDG_STATE_HOME` point at a temporary directory, as in
the live tests above.

`herdr plugin log list --plugin dev.codemap` shows each action and hook run
with its output.

## Adding a language

Language support lives in `src/lang/`: a tree-sitter grammar crate, queries for
functions, classes, calls and imports (`src/lang/queries.rs`), and import
resolution (`src/lang/imports.rs`). Flowcharts are built in `src/flow.rs`. Add
fixtures and cases to `tests/index.rs` and `tests/flow.rs` alongside it, and
check the grammar's license for NOTICE.

## Commits and pull requests

`main` changes only through pull requests, for everyone, maintainers
included. A pull request can merge when:

- the `CI gate` and `PR title` checks are green on its latest commit;
- the branch is up to date with `main` (use **Update branch**, or
  `gh pr update-branch`);
- every review conversation is resolved.

Pull requests are squash-merged, so `main` has one commit per pull request,
signed by GitHub, and the pull request title becomes its message. **Enable
auto-merge** (`gh pr merge --auto --squash`) to merge as soon as the checks
pass. Force pushes and deletion of `main` are blocked, and release tags
(`v*`) cannot be moved or deleted.

- Title the pull request as a
  [Conventional Commit](https://www.conventionalcommits.org/): `feat:`,
  `fix:`, `docs:`, `test:`, `refactor:`, `perf:`, `build:`, `ci:`, `chore:`,
  with an optional scope, as in `fix(flow): keep the gutter visible`. The
  `PR title` check enforces it.
- Write commits the same way, and keep each to one logical change that builds
  and passes the tests.
- Add a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) for
  anything a user would notice.
- Behaviour changes come with a test. UI changes come with updated snapshots
  and screenshots.
- Keep examples, fixtures and screenshots free of private code, paths and
  names.
- Do not change action ids, pane ids, event hooks or command paths in
  `herdr-plugin.toml` without discussing it first: people's key bindings refer
  to them.

## Releases

Maintainers release by bumping `version` in `Cargo.toml` and
`herdr-plugin.toml`, moving the Unreleased notes into a dated section of the
changelog, and pushing a `vX.Y.Z` tag. The release workflow checks that the
three agree, builds macOS and Linux binaries for x86_64 and aarch64, and
publishes the GitHub release with that changelog section as its notes.
