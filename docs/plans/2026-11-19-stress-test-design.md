# Kanban Stress Testing — Design

Date: 2026-11-19. Order chosen by the user: performance first, then data
robustness, then headless UI tests.

## 1. Rendering performance (measure, then fix)

- `--stress N` builds a deterministic board of N cards (seeded; titles mix short,
  long, emoji, accented, CJK and right-to-left text) and saves to
  `gpui-kanban/stress.json`, never the real `board.json`.
- `--chaos` starts chaos mode immediately; `--exit-after SECS` quits on a timer.
  Together they make benchmark runs scriptable (`scripts/bench.sh`).
- In stress mode the window redraws every frame (`request_animation_frame`), so
  the gap between renders is the true frame time. Shown in the header and
  printed once per second as `[perf] …`: FPS, average and worst frame gap over the
  last second, element-build time inside `render()`, duration of the last save.
- Chaos mode (button or flag): every frame moves 20 random cards, and sometimes
  adds or deletes one; each change is saved, as in normal use.
- Record a baseline at 1k / 10k / 100k cards (idle and chaos, release build) in
  `docs/stress-results.md`, then fix only what the numbers show. Likely
  candidates: `uniform_list` virtualization (cards become single-line,
  truncated), debounced background saving, and O(N) `Vec::remove` in `move_to`.
  Re-measure after each fix.
- The window must stay visible while benchmarking: GPUI stops drawing occluded
  windows.

## 2. Data robustness (proptest)

Model-based tests: random sequences of add/delete/move (messy titles: blank,
huge, emoji, RTL, unknown ids) applied to `Board` and a trivially simple
reference model; check invariants after each step; save/load round-trips; random
bytes and truncated JSON never panic the loader and are preserved in `.bak`.

## 3. Headless UI tests (`gpui` `test-support`, verified to build)

`KanbanBoard` takes its board and save path as parameters; elements get
`debug_selector`s. Tests: typing/Enter/Backspace/Escape, × delete, drag to
another column, drop on own column, and a 1,000-random-drag soak on 10k cards.

Parts 2 and 3 are planned in detail after part 1, since part 1 changes the code
they test.
