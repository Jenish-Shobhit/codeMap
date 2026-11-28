---
name: selection-scroll
description: "Formal verification (Kani) of how the Map and Flow views scroll to keep the selected box on screen: `follow` in src/ui/mod.rs. Use before and after any change to `follow`, to its callers in src/ui/map_view.rs or src/ui/flow_view.rs, to scroll margins, or to how box rectangles reach `follow`; and when the Kani CI job fails."
---

# Selection scroll: the selected box stays on screen

`follow` (src/ui/mod.rs) decides where the Map and Flow views scroll when the
selection moves. Kani proves two promises about it over every input in the
bounds below, on the real function, not a copy:

| Harness | Promise | Harm if broken |
| --- | --- | --- |
| `ui::proofs::follow_shows_a_box_that_fits` | A box that fits the viewport with `follow`'s scroll margin (2 columns, 1 row) and lies inside the canvas is entirely visible after `follow`, wherever the view was scrolled before. | Moving the selection leaves the selected box partly or wholly off screen. |
| `ui::proofs::follow_never_scrolls_past_the_content` | The view never scrolls past the right or bottom edge of the canvas. | The view scrolls into empty space. |

## Does my change touch the verified code?

Coverage record (sync commit `4c75657`, the commit at which these harnesses
were last confirmed to describe the code):

- src/ui/mod.rs: `follow` (the whole function) and the `#[cfg(kani)] mod proofs`
  at the end of the file.
- src/ui/map_view.rs: the `follow(...)` call in `render`, which passes a
  layout node's `(x, y, w, h)` and the canvas size.
- src/ui/flow_view.rs: the `follow(...)` call, which widens the selected box by
  `crate::flow::GUTTER` to the left, and the line after it that resets
  `scroll.0` to 0 when the chart fits.
- src/canvas.rs: `Canvas::new`, whose `w * h` allocation bounds canvas sizes.

To see what changed since the sync commit:

```sh
git log -p 4c75657..HEAD -- src/ui/mod.rs src/ui/map_view.rs src/ui/flow_view.rs src/canvas.rs
```

Also look for new callers (`git grep -n 'follow('`) and for new code that
writes `app.map.scroll` or `app.flow.scroll` without going through `follow`:
that code is outside the proof.

## Run the check

Kani 0.68.0 (the version CI pins in `.github/workflows/ci.yml`):

```sh
cargo install --locked kani-verifier --version 0.68.0   # once
cargo kani setup                                         # once
python3 scripts/kani.py prove       # each harness holds and reaches every cover
python3 scripts/kani.py mutations   # each broken variant in src/ui/mod.mutations is caught
```

Both take seconds. `prove` prints `PASS <harness> (n of n covers)` per harness;
`mutations` prints `DETECTED` per broken variant and ends with
`SUMMARY 5 detected, 0 missed, 0 errors`. CI runs both in the `Kani proofs`
job, and the `CI gate` check that main requires fails when they do.

## Bounds and assumptions

- The viewport `(vw, vh)` is a terminal area: each side is 1 to `u16::MAX`. Both
  callers pass `Rect` sides. `follow` returns early for an empty viewport, so
  the proofs assume a non-empty one.
- Canvas sides, box coordinates and the old scroll position are at most 2^20
  cells, far beyond any canvas `Canvas::new` could allocate.
- `follow_shows_a_box_that_fits` assumes `w + 2 <= vw`, `h + 1 <= vh`,
  `x + w <= canvas width` and `y + h <= canvas height`. Nothing else is assumed.

## What the proofs do not cover

- **A box nearly as wide or tall as the viewport.** For `w + 2 > vw` the proof
  says nothing, and the code does hide such a box's left edge: a 19-wide box at
  x=10 in a 20-wide view of a 100-wide canvas leaves the view starting at x=11
  (Kani's counterexample: `w = vw - 1`). The `+ 2` right margin wins over the
  box's left edge. The same holds vertically for `h + 1 > vh` with the `+ 1`
  margin. This is a known finding, not fixed yet; when it is fixed, add a
  harness for the wide case (`assert!(scroll.0 <= x)` for `w <= vw`) and a
  mutation that restores the old margin.
- Drawing: `blit` and the terminal's own cell widths. A box that `follow`
  places on screen can still render wrongly if its glyphs are wider than
  `util::char_width` says.
- Callers that change `scroll` after `follow`, such as the Flow view's reset to
  0 when the chart fits; that reset can only reveal more of the chart.

## When a check fails

- **`prove` fails with a counterexample.** Rerun the harness with
  `cargo kani --lib --harness <name> --exact --concrete-playback=print -Z concrete-playback`
  to get concrete inputs. Replay them against the real function (a unit test
  calling `crate::ui::follow` with those values). If the inputs are possible
  for a caller (a terminal-sized view, a box inside the canvas), it is a bug in
  the code: report it with the inputs as a short story ("box at x, view
  scrolled to s, after follow the view starts at s'"), and do not weaken the
  assertion or add assumptions to make it pass. If no caller can produce the
  inputs, the harness is missing a real constraint: add it as an assumption
  with the source line that guarantees it.
- **`prove` fails with a missing or unsatisfied cover.** The assumptions now
  exclude the situation the cover names, so the proof may hold vacuously. Fix
  the assumptions, not the cover.
- **`mutations` reports MISSED.** A broken variant now passes: the harness has
  lost its ability to catch that bug, usually because an assumption or bound
  changed. Restore the strength of the harness.
- **`mutations` reports ERROR "must occur exactly once".** `follow` was edited
  and a mutation's `-` line no longer matches. Rewrite the mutation against the
  new code so it breaks the same promise.

## When the promise changes on purpose

If a change to `follow` alters what it promises (a different margin, centring
instead of edge-following), cite the evidence (doc comment, test, changelog
entry or commit message), update the harness and this file in the same pull
request, rerun both commands, and add a mutation for each new branch. Move the
sync commit above forward only after confirming that the harnesses describe
the code at the new commit.
