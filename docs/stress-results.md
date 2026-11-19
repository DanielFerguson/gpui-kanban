# Stress test results

Machine: MacBook Pro, built-in Liquid Retina XDR (ProMotion, 120 Hz), macOS 27.0.1.
Release build, 1000×680 window. Numbers are the steady state from
`scripts/bench.sh` (last of three once-a-second `[perf]` lines).

- **fps / avg / worst**: frames in the last second and the gap between them.
  This is the number that matters. The ceiling here is 120 fps / 8.3 ms
  (measured with a 3-card board).
- **build**: time inside `render()` building the element tree. It excludes
  GPUI's layout and paint, which happen afterwards.
- **save**: time to write the board to JSON on the last change.
- **chaos**: 20 random moves per frame, plus about one add and one delete.

## Baseline: every card is a laid-out element

| Cards | Mode | fps | avg frame | worst | build | save |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | idle | 53 | 19.0 ms | 21.3 ms | 0.8 ms | – |
| 1,000 | chaos | 49 | 20.4 ms | 22.0 ms | 1.3 ms | 0.9 ms |
| 10,000 | idle | 5 | 203.8 ms | 207.3 ms | 9.7 ms | – |
| 10,000 | chaos | 5 | 206.5 ms | 208.1 ms | 11.3 ms | 1.7 ms |
| 100,000 | idle | 0.2 | 5,145 ms | 5,145 ms | 252 ms | – |
| 100,000 | chaos | 0.2 | 4,414 ms | 4,414 ms | 254 ms | 17.6 ms |

Findings:

1. Frame time grows linearly with the total number of cards: about 20 µs per
   card at 1k and 10k, and about 50 µs at 100k. Most cards are scrolled out of
   view, but GPUI still lays out and paints all of them. Only about 5% of the
   frame is our own `render()`; the rest is layout and paint.
2. Even 1,000 cards misses the 120 fps ceiling by more than 2×.
3. Saving is not the bottleneck yet. At 100k cards it takes about 17 ms, which
   is a whole 120 Hz frame by itself, so it will matter once rendering is fixed.

## Fix 1: virtualized columns (`uniform_list`)

Each column became a `uniform_list`, so GPUI only builds, lays out and paints
the rows in view. Cards are now one line (`truncate()`), because a
`uniform_list` needs every row to be the same height; the drag preview still
shows the full title. Column contents come from `Board::index_columns`, which
is rebuilt every frame (0.2 ms at 100k cards).

| Cards | Mode | fps | avg frame | worst | save (UI thread) |
|---:|---|---:|---:|---:|---:|
| 1,000 | idle | 119 | 8.4 ms | 14.7 ms | – |
| 1,000 | chaos | 120 | 8.3 ms | 8.7 ms | 0.5 ms |
| 10,000 | idle | 120 | 8.3 ms | 9.0 ms | – |
| 10,000 | chaos | 119 | 8.4 ms | 16.4 ms | 1.9 ms |
| 100,000 | idle | 120 | 8.3 ms | 9.0 ms | – |
| 100,000 | chaos | 61–67 | 15.0–16.5 ms | 67.5 ms | 12.8 ms |

Everything hits the ceiling except 100k chaos, where the synchronous save now
dominates. One-off timing of a 100k-card save (12 MB of JSON):

| Step | Time |
|---|---:|
| chaos step (20 moves, `Vec::remove`) | ~1.2 ms |
| `index_columns` | ~0.2 ms |
| clone the board | ~2.2 ms |
| pretty JSON | ~12 ms |
| compact JSON | ~8.5 ms |
| write file (warm) | ~3–4 ms |

## Fix 2: throttled background saving

The UI thread now only takes a numbered snapshot (a clone, ~2 ms at 100k). A
background thread converts it to JSON and writes it, at most once per 250 ms;
changes made in between are folded into the next save. On Cmd-Q
(`on_app_quit`) and on window close (`on_release`), any pending change is saved
immediately. `Saver` makes sure an older snapshot can never overwrite a newer
one if two writes overlap.

| Cards | Mode | fps | avg frame | worst | bg save (off UI thread) |
|---:|---|---:|---:|---:|---:|
| 1,000 | idle | 119 | 8.5 ms | 16.5 ms | – |
| 1,000 | chaos | 117–120 | 8.3–8.6 ms | 9.2–19.2 ms | 0.4–8.3 ms |
| 10,000 | idle | 120 | 8.3 ms | 9.3 ms | – |
| 10,000 | chaos | 120 | 8.3 ms | 9.2 ms | 1.9 ms |
| 100,000 | idle | 120 | 8.3 ms | 9.3 ms | – |
| 100,000 | chaos | 120 | 8.3 ms | 9.4 ms | 15–20 ms |

Every configuration now runs at the display's 120 Hz ceiling. Compared with the
baseline, that is 2.3× faster at 1k cards, 24× at 10k, and about 600× at 100k.

Not changed, because the numbers don't call for it: `Vec::remove` in
`move_to`/`delete` (~1.2 ms per chaos frame at 100k) and pretty-printed JSON.

# Part 2: Data robustness (property-based fuzzing)

`src/board/fuzz.rs` and the property tests in `src/saver.rs` use
[proptest](https://docs.rs/proptest). Each property runs 256 random cases on
`cargo test`; set `PROPTEST_CASES=10000` for a soak run. When a property
fails, proptest shrinks the input to a minimal example.

| Property | Input |
|---|---|
| Random operations match a model | 0–200 adds, deletes, moves and save/reloads per case, using messy titles (any Unicode, whitespace-only, padded, 3,000-character, ZWJ emoji, RTL, combining marks, NUL, quotes). Checked after every step against a deliberately different model: three per-column lists. |
| Loading any bytes is safe | 0–2,000 random bytes |
| Loading a truncated save is safe | a valid save cut at a random byte |
| Loading a corrupted save is safe | a valid save with 1–3 bytes replaced by digits |
| Loading boards with any ids is safe | repeated ids, `u64::MAX`, anything |
| Saver: any write order leaves the newest on disk | 1–8 snapshots written in a shuffled order |
| Saver: concurrent writes leave the newest on disk | 1–8 snapshots written from separate threads at once |

"Safe" means `load` doesn't panic; the result has unique ids, a `next_id`
above them all, and consistent columns; adding a card still works; and after
the app's next save, any file that couldn't be used is still there,
byte-for-byte, in `board.json.bak`.

## Bugs found

1. **Silent data loss on a non-UTF-8 file.** A save cut off mid-emoji, or one
   re-saved by an editor in another encoding, took the "couldn't read" path,
   which started a fresh board without backing the file up. The next save then
   overwrote it. Found by three properties at once.
   *Fix:* read raw bytes and let the JSON parser reject bad UTF-8, so every
   unusable file is moved to `.bak`.
   *Regression test:* `load_invalid_utf8_backs_it_up_and_starts_fresh`.
2. **Crash on launch.** A card id of `u64::MAX` overflowed when `load`
   computed `max_id + 1`. In a release build it wraps to 0 instead, and ids
   start being reused.
3. **Crash on the next new card.** A `next_id` of `u64::MAX` overflowed in
   `add`.
4. **Deleting one card deleted another.** `load` accepted repeated ids, and
   `delete` removes every card with the given id.
   *Fix for 2–4:* on load, if ids repeat or exceed 2^53 − 1 (the largest
   integer every JSON tool agrees on), renumber the cards. Ids are never shown,
   so this is invisible.
   *Regression tests:* `load_survives_the_largest_possible_ids` and
   `load_renumbers_duplicate_ids`.

The first version of the ids property used plain random `u64`s and passed,
because random 64-bit numbers almost never collide or hit the maximum.
Weighting the generator towards small, colliding ids and the extremes exposed
bugs 2–4 straight away.

The board operations themselves (add, delete, move, save and reload) and the
`Saver`'s ordering guarantee found no bugs.

## Soak run

`PROPTEST_CASES=10000 cargo test --release`: all 28 tests pass, about 100,000
random scenarios in 47 s. Release builds disable overflow checks, so the same
run was repeated in a debug build, where overflow panics: `PROPTEST_CASES=10000
cargo test` also passes all 28 tests (155 s).

## Known limits

- A second unusable file overwrites an earlier `board.json.bak`.
- Titles from a hand-edited file are loaded as they are, even blank or
  untrimmed ones. Only the structure (ids) is repaired.

# Part 3: Headless UI tests

`src/ui_tests.rs` drives the real `KanbanBoard` view with GPUI's test harness
(`gpui` with the `test-support` feature, as a dev-dependency only). Input is
simulated mouse and keyboard events; elements are found with
`debug_selector`s (`card-ID`, `delete-ID`, `column-N`, `input-N`), which
compile to nothing outside tests. Time is a fake clock that only moves when a
test calls `advance_clock`, so the 250 ms save delay is tested exactly.

| Test | Checks |
|---|---|
| typing + Enter | the card is added to that column; the draft clears |
| Backspace, Escape | editing works; Escape clears the field and leaves it; nothing is added |
| blank Enter | adds nothing |
| shortcuts | `cmd-c` and `ctrl-a` aren't typed into the field |
| × | deletes exactly that card |
| drag to another column | the card moves to the bottom of that column |
| drop on its own column | nothing changes |
| saving | nothing is written before 250 ms; the board is written after |
| quick changes | 5 rapid adds produce 1 snapshot |
| quit | a pending change is saved immediately (`on_app_quit`) |
| close window | a pending change is saved immediately (`on_release`) |
| soak | random drags (80%), × deletes (10%) and typed adds (10%) on a 10,000-card board; the view matches an expected board after every step |

The soak runs 1,000 steps in release builds (`cargo test --release`, ~5 s)
and 100 in debug builds, where rendering is about ten times slower.
`SOAK_STEPS=N` overrides either. 1,000 steps pass in both builds.

The two save-on-exit tests were checked against deliberate breakage. With the
quit hook disabled, the quit test fails; with the window-close hook disabled,
the close test fails; each still passes when only the other hook is disabled.

## Findings

No bugs in the app. Drag and drop, deleting, typing, throttled saving, and
saving on quit and on close all behave as designed. Drag and drop also works
inside the virtualized lists, through 1,000 random operations on 10,000 cards.

One crash in GPUI 0.2.2's test harness: calling `quit()` through the
`VisualTestContext` returned by `add_window_view` crashes the test process with
SIGSEGV. `into_mut` leaks that context and registers a quit hook that frees it,
so `quit()` frees the context it is running on and then reads
`self.app` from freed memory (`test_context.rs:170`; GPUI's own comment there
says "This is unsound"). The backtrace confirmed it, and the crash happens with
an empty view too, so it isn't caused by this app. Workaround: call `quit()` on
the outer `TestAppContext` instead.

## Test suite

40 tests: 21 unit (16 board, 3 saver, 2 command-line options), 7 property-based (5 board, 2 saver), 12 UI.

| Command | Time |
|---|---:|
| `cargo test` (debug, 256 cases per property, 100 soak steps) | ~6 s |
| `cargo test --release` (1,000 soak steps) | ~5 s |
| `PROPTEST_CASES=10000 cargo test --release` | ~47 s |
