# Stress Testing — Part 1 (Performance) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Measure how the GPUI kanban board behaves with 1k–100k cards and constant changes, then fix the bottlenecks the numbers reveal.

**Architecture:** Stress data and chaos live in `src/board.rs` (testable). `src/main.rs` gains CLI options, a `Perf` frame-timing struct, a chaos toggle, and continuous redraw in stress mode. `scripts/bench.sh` runs the matrix against the release build.

**Tech Stack:** `gpui` 0.2.2, `fastrand` 2 (seeded RNG).

Design: `docs/plans/2026-11-19-stress-test-design.md`.

---

### Task 1: Stress data and chaos in the model (TDD)

**Files:** Modify `Cargo.toml` (add `fastrand = "2"`), `src/board.rs`.

1. Write failing tests:
   - `stress_board_is_deterministic_and_well_formed` — `stress(1000, 7) == stress(1000, 7)`, 1000 cards, unique ids, `next_id == 1000`, every column non-empty.
   - `chaos_keeps_the_board_consistent` — 1,000 `chaos` steps on `stress(100, 1)`: ids unique, `next_id` > every id; also on an empty board (no panic).
2. `cargo test` → both fail (`todo!`).
3. Implement `Board::stress(count, seed)` (titles from a fixed list, prefixed `#i`, random column) and `Board::chaos(&mut rng, moves)` (move `moves` random cards to random columns; 50% add one; 50% delete one). Add `data_file(name)`; `default_path()` uses it.
4. `cargo test` → all pass.

### Task 2: Stress mode, perf readout, chaos toggle

**Files:** Modify `src/main.rs`.

1. `Options { stress: Option<usize>, chaos: bool, exit_after: Option<Duration> }` from `std::env::args()`; unknown args → usage on stderr, exit 2.
2. `Perf { frames: VecDeque<(Instant, Duration)>, last_frame, build, save, last_report }`:
   - `frame_started()` pushes the gap since the previous render, drops entries older than 1 s.
   - `summary(cards, chaos)` → `"10000 cards · 118 fps · avg 8.4 ms · worst 12.1 ms · build 1.2 ms · save 6.3 ms · chaos"`.
   - Printed as `[perf] …` at most once per second.
3. `KanbanBoard::new(board, save_path, stress, chaos, window, cx)`. `save()` is timed; `changed()` = `save()` + `notify()`.
4. `render()`: if stress → `frame_started()`, `window.request_animation_frame()`; if chaos → `board.chaos(rng, 20)` + `save()`; build tree, record build time. Header shows the summary and a ▶ Chaos / ■ Stop button.
5. `main`: stress → `Board::stress(n, 42)` + `data_file("stress.json")`; `--exit-after` → spawn timer then `cx.quit()`.
6. `cargo build` clean; `cargo test` passes.

### Task 3: Baseline

**Files:** Create `scripts/bench.sh`, `docs/stress-results.md`.

1. `bench.sh` builds `--release`, then for N in 1000, 10000, 100000 and mode in idle, chaos runs `--stress N [--chaos] --exit-after 6` and keeps the last 3 `[perf]` lines.
2. Run it (window must stay visible); record the table in `docs/stress-results.md`.

### Task 4+: Fixes, driven by the baseline

For each bottleneck, in order of impact: change, `cargo test`, re-run the affected rows, record before/after. Candidates:
- **Layout of every card each frame** → `uniform_list` per column over cached per-column index lists; fixed-height single-line cards (`truncate()`).
- **Synchronous save on every change** → mark dirty; debounced (250 ms) save; serialize + write on `background_spawn`.
- **`Vec::remove` in `move_to` / `delete`** (O(N) memmove) → only if it shows in the numbers.

---

# Part 2: Data robustness (proptest)

**Goal:** Throw random operation sequences and random files at `Board` and `Saver`, and fix whatever breaks.

**Files:** `Cargo.toml` (dev-dependency `proptest = "1"`); create `src/board/fuzz.rs` (declared `#[cfg(test)] mod fuzz;` in `src/board.rs`); add a property to `src/saver.rs`.

### Task 5: Model-based operation fuzzing
- Strategies: `status()`; `title()` mixing printable Unicode, whitespace-only (including non-breaking and em spaces), padded, huge (up to 3,000 chars of any `char`, control characters included) and hand-picked nasties (ZWJ emoji, RTL, combining marks, `\0`, quotes, backslashes); `target()` = an existing card or any id 0..300.
- `Op::{Add, Delete, Move, SaveAndLoad}`, 0–200 per case.
- `Model`: the board kept as three separate per-column lists, a deliberately different representation from `Board`'s single list.
- After every op: return values match; every column matches the model in order; `next_id` matches; `check_structure` (unique ids, `next_id` above them all, `index_columns` covers every card exactly once in the right column); titles non-empty and trimmed.

### Task 6: Files from the outside world
- Any bytes (0–2,000), truncated valid JSON, digit-mutated valid JSON, and boards with arbitrary `u64` ids and `next_id`.
- Property: `load` never panics; the result passes `check_structure`; adding a card afterwards still passes; and after load and then save, a file that didn't parse is preserved byte-for-byte in `.bak`.

### Task 7: Saver ordering
- 1–8 snapshots of a growing board, written in a shuffled order (and concurrently from threads): the file always ends up equal to the newest snapshot.

Each failing property becomes a fix plus a named regression test. Results go in `docs/stress-results.md`.

---

# Part 3: Headless UI tests

**Goal:** Drive the real `KanbanBoard` view with simulated mouse and keyboard input, using GPUI's test harness, with no window on screen.

**Files:** `Cargo.toml` (dev-dependency: `gpui` with `test-support`); `src/main.rs` (`debug_selector`s `column-N`, `card-ID`, `delete-ID`, `input-N`, which compile to nothing outside tests); create `src/ui_tests.rs` (`#[cfg(test)] mod ui_tests;`); `src/saver.rs` (test-only `snapshots_taken()`).

### Task 8: Interaction tests
Helpers: `open(cx, board)` gives a view saving to a temp dir; `click(selector)`; `drag(from, to)` (mouse down, move >2 px to start the drag, move to the target, mouse up); `board(view)`.
- Typing + Enter adds to that column; Backspace; Escape clears and unfocuses; blank Enter adds nothing; cmd-shortcuts aren't typed.
- × deletes exactly that card.
- Drag to another column moves the card to the bottom; drop on its own column changes nothing.

### Task 9: Saving through the UI (fake clock)
- Nothing is written before `SAVE_INTERVAL`; it's written after `advance_clock(SAVE_INTERVAL)`.
- Five quick changes produce one snapshot.
- Quit (`TestAppContext::quit`) and window close each save a pending change immediately.

### Task 10: Soak
- 10k-card board, 1,000 random UI operations (80% drag, 10% delete via ×, 10% add via typing) on cards in the top rows; after each one, the view's board equals an expected board updated with the same operations.
