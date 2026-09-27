## Summary

<!-- What this changes and why. Link the issue it closes, if any. -->

## Testing

<!-- How you checked it. For UI changes, paste the relevant snapshot diff or a frame. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --locked`
- [ ] Behaviour changes have a test
- [ ] UI changes: snapshots updated with `UPDATE_SNAPSHOTS=1 cargo test --test views` and reviewed
- [ ] UI changes: screenshots regenerated with `cargo run --example screenshots`
- [ ] `CHANGELOG.md` has a line under `[Unreleased]` for user-visible changes
- [ ] No action ids, pane ids, event hooks or command paths changed in `herdr-plugin.toml`
- [ ] No private code, paths or names in fixtures, examples or screenshots
