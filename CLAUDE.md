# codeMap

A herdr plugin and standalone TUI in Rust: Changes and History views of a
repository. CONTRIBUTING.md has the full setup and test guide.

## No comments in code

Do not write comments in code. In Rust that means no `//`, `/* */`, `///` or
`//!`; in Python, no `#` comments (a shebang is fine) and no docstrings. Say it
in the code instead: clear names, types, small functions and tests.

`tests/no_comments.rs` enforces this: `cargo test` and the CI gate fail on any
comment in `src/`, `tests/`, `examples/` or `scripts/`. `tests/fixtures/` is
parser input, not our code, and is exempt.

Explanation that matters goes in the commit message, the pull request
description, CONTRIBUTING.md or README.md, or a project skill in
`.claude/skills/`, never in the code.

## How changes land

- `main` takes changes only through pull requests. Never push to `main`.
- Title the pull request as a Conventional Commit (`fix(changes): ...`); it
  becomes the squash commit on `main`.
- A pull request merges when `CI gate` and `PR title` are green and the branch
  is up to date with `main`.

Before pushing:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

- UI changes: `UPDATE_SNAPSHOTS=1 cargo test --test views`, review the diff,
  and `cargo run --example screenshots`.
