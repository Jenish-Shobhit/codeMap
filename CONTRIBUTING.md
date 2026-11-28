# Contributing to codeMap

Thanks for helping. Bug reports, fixes and documentation are all welcome. For a larger change, open an issue first so we can agree on
the approach before you write it.

By contributing you agree that your work is licensed under the
[MIT License](LICENSE), and to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Development setup

You need:

- **Rust**: `rust-toolchain.toml` selects stable with rustfmt and clippy, and
  rustup installs it on first use. The minimum supported version is 1.90
  (`rust-version` in `Cargo.toml`).
- **A C compiler**, for the tests only: `tests/no_comments.rs` finds comments
  with tree-sitter, whose grammars are C. Xcode's command line tools on macOS;
  `gcc` or `clang` on Linux.
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

CI also runs the test suite on Rust 1.90, `cargo deny check` (deny.toml),
actionlint and zizmor on the workflows, and checks that the README screenshots
are current. A last job, `CI gate`, passes only when
every other job passed; it is the check `main` requires.

## Tests

| File | What it covers |
| --- | --- |
| `tests/views.rs` | golden frames of every view and their states, colours and keys |
| `tests/git_store.rs`, `tests/hook.rs` | turn diffs from checkpoints, the side store writing nothing into the repository, the graph, worktrees, the hook |
| `tests/git_status.rs` | what Status reads as staged, not staged and untracked, that reading it leaves `.git` untouched, and that many reads at once each see the same index |
| `tests/git_worktree.rs` | the worktrees of a repository with their branch, upstream, locked and missing states and uncommitted changes, the agents herdr lists placed in them, and that reading them writes into none |
| `tests/git_write.rs` | staging, unstaging and discarding files and single hunks, commits and the ways they fail, each on a temporary repository |
| `tests/git_history.rs` | the history of one file across a rename, names with spaces, glob characters, a leading dash and accents passed literally, the per-file diff of each commit, files with no commits, and which commits a branch on `origin` holds |
| `tests/git_blame.rs` | blame read from real repositories: several commits and authors, lines not committed yet, renames, tabs, CRLF, wide characters, control characters in authors and subjects, awkward file names, the files with nothing to blame, that nothing in the output reaches git as an option, and that reading writes nothing |
| `tests/git_branch.rs` | listing branches against a local bare `origin`, the default branch, switching, creating, merging and deleting, and the count of commits a deletion would lose |
| `tests/gh.rs` | reading pull requests: fixture JSON, and a fake `gh` that prints it, fails, prints something else or outlives the timeout |
| `tests/manifest.rs` | `herdr-plugin.toml` against herdr 0.9.0's link rules |
| `tests/pty.rs` | the real binary on a pseudo-terminal: keys, Esc, first-paint time, staging and committing a file, creating and deleting a branch, jumping to another worktree and back |
| `tests/no_comments.rs` | no comments anywhere in our Rust or Python code |

No test runs the real `gh` or touches the network. The Branches tab takes the
program to run from `Options::gh`, which only `main` fills from `CODEMAP_GH`.
Tests pass a path that does not exist (`no_gh()`), or a shell script they
write into a temporary directory (`fake_gh`, `gh_printing` in
`tests/common/mod.rs`); the pseudo-terminal helper sets `CODEMAP_GH` the same
way for the binary it starts.

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
cargo test --release --test perf -- --ignored --nocapture        # history of 2,000 commits
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
  with an optional scope, as in `fix(changes): keep the gutter visible`. The
  `PR title` check enforces it.
- Write commits the same way, and keep each to one logical change that builds
  and passes the tests.
- No comments in code: no `//`, `/* */`, `///` or `//!` in Rust, no `#`
  comments or docstrings in Python. Let names, types and tests carry the
  meaning; put explanation in the commit message, the pull request, the docs
  or a project skill. `tests/no_comments.rs` fails the build on any comment in
  `src/`, `tests/`, `examples/` or `scripts/` (`tests/fixtures/` is exempt).
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
