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
- It runs `git` with a private `GIT_DIR` and index in its plugin state
  directory (by default `~/.local/state/herdr/plugins/dev.codemap/`). It
  never writes objects, refs or index entries into your repository.
- It connects to the herdr socket named by `HERDR_SOCKET_PATH` and reads
  herdr's `config.toml` for the theme.
- It sends text to an agent's pane only when you press `P` or `S` in the
  Changes view.

Problems in those areas are in scope: writes into a repository, command or
argument injection through file names, branch names or diffs, reading
outside the opened repository, or text reaching an agent without a keypress.

Vulnerabilities in herdr itself belong to
[herdr's security process](https://github.com/herdrdev/herdr/security).
