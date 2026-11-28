# Security policy

## Supported versions

Security fixes land on `main` and ship in the next release. Only the latest
release is supported.

| Version | Supported |
| --- | --- |
| 0.1.x | Yes |

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately through GitHub's
[private vulnerability reporting](https://github.com/Jenish-Shobhit/codeMap/security/advisories/new)
for this repository. Include:

- the codeMap version (`codemap --version`) and herdr version (`herdr --version`);
- your operating system;
- what an attacker can do, and the steps or a minimal repository that shows it.

Leave out private code, repository paths and terminal output that you would
not want disclosed. You should get a first response within a week. Once a fix
is ready, it is released and the advisory is published with credit to you,
unless you prefer otherwise.

## Scope

codeMap runs as your user, with the permissions of the herdr plugin that
launches it. What it touches:

- It reads the files and git metadata of the repository you open.
- To read, and to record turns, it runs `git` with a private `GIT_DIR` and
  index in its plugin state directory (by default
  `~/.local/state/herdr/plugins/dev.codemap/`). Reading and recording never
  write objects, refs or index entries into your repository.
- It writes into your repository only from three keys in the Status tab and
  four in the Branches tab. In Status, `space` changes the index, `c` runs
  `git commit` and with it your hooks and signing program, and `x`, after you
  confirm with `y`, restores or deletes files in the working tree. In
  Branches, `⏎` runs `git switch`, `n` runs `git switch -c` with the name you
  typed, and `m` and `D`, each after you confirm with `y`, run `git merge
  --no-edit` and `git branch -D` on the selected branch.
- It never pushes, pulls or fetches, and never creates, changes or deletes
  anything on a remote.
- It connects to the herdr socket named by `HERDR_SOCKET_PATH` and reads
  herdr's `config.toml` for the theme. codeMap itself opens no network
  connection.
- When the Branches tab is first shown it runs `gh pr list` in the
  repository, if `gh`, or the program `CODEMAP_GH` names, is installed. `gh`
  contacts GitHub with your `gh` login; codeMap gives it no input, stops it
  after five seconds and reads what it prints. `B` opens the pull request
  address `gh` printed only when it starts with `https://`.
- It sends text to an agent's pane only when you press `P` or `S` in the
  Changes view.

Problems in those areas are in scope: a write into a repository without one
of those keys or beyond the selected file, hunk or branch, a branch deleted
when the question said nothing would be lost, command or argument injection
through file names, branch names, diffs, commit messages or what `gh` prints,
`gh` running before the Branches tab is shown, reading outside the opened
repository, or text reaching an agent without a keypress.

Vulnerabilities in herdr itself belong to
[herdr's security process](https://github.com/herdrdev/herdr/security).
