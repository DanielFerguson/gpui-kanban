# gpui-kanban

A small kanban board for macOS built with [GPUI](https://gpui.rs) 0.2.2, the
GPU-accelerated UI framework behind the Zed editor. It also includes a stress
test that pushes the board to 100,000 cards.

- Three columns: To Do, Doing, Done
- Drag cards between columns
- Add cards by typing in a column's field and pressing Enter; delete them with ×
- Saves automatically to `~/Library/Application Support/gpui-kanban/board.json`

## Running it

You need macOS, Rust (2024 edition; tested with 1.89) and Xcode.

```sh
cargo run                    # the kanban board
cargo run --example hello    # GPUI's hello world
```

GPUI is built with its `runtime_shaders` feature, which compiles its Metal
shaders when the app starts. Recent Xcode versions ship the shader compiler as
a separate "Metal Toolchain" download; with this feature you don't need it.

## Stress mode

```sh
cargo run --release -- --stress 100000           # a generated 100k-card board
cargo run --release -- --stress 100000 --chaos   # ...moving 20 cards every frame
scripts/bench.sh                                 # 1k / 10k / 100k, idle and chaos
```

Stress boards save to `stress.json`, never your real board. The header shows
frames per second, frame times and save times, and the same numbers are
printed once a second.

Results on a 120 Hz MacBook Pro:

| Cards | Before | After |
|---:|---:|---:|
| 1,000 | 53 fps | 120 fps |
| 10,000 | 5 fps | 120 fps |
| 100,000 | 0.2 fps | 120 fps (also with chaos) |

The two fixes were virtualized columns (`uniform_list`, so only visible cards
are laid out) and throttled background saving. The full write-up, including
four save-file bugs found by fuzzing, is in
[docs/stress-results.md](docs/stress-results.md).

## Tests

```sh
cargo test             # 40 tests, ~6 s
cargo test --release   # same, with the full 1,000-step UI soak
PROPTEST_CASES=10000 cargo test --release   # long fuzzing run
```

- **Unit tests** cover the board model, saving and loading, and command-line options.
- **Property-based tests** ([proptest](https://docs.rs/proptest)) run random
  operation sequences against a reference model and feed random, truncated and
  corrupted save files to the loader.
- **Headless UI tests** use GPUI's test harness to click, type and drag in the
  real view, with no window shown.

## Layout

| Path | What |
|---|---|
| `src/main.rs` | The GPUI app: window, columns, cards, text field, stress mode |
| `src/board.rs` | The board model, loading and saving (no UI code) |
| `src/saver.rs` | Background saving that never lets an older snapshot win |
| `src/perf.rs` | Frame timing for stress mode |
| `src/board/fuzz.rs` | Property-based tests |
| `src/ui_tests.rs` | Headless UI tests |
| `docs/` | Design notes, plans and stress-test results |
