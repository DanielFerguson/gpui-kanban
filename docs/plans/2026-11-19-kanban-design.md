# GPUI Kanban Board — Design

Date: 2026-11-19

## Goal

A "hello world"-sized desktop app on macOS built with the official `gpui` crate
(0.2.2, https://gpui.rs), shaped as a simple kanban board.

## Scope

- Three fixed columns: To Do, Doing, Done, seeded with sample cards on first run.
- Drag & drop cards between columns.
- Add cards (per-column text field) and delete cards (× button).
- Persist to JSON; survives restarts.

Out of scope: reordering within a column, editing card titles, multiple boards,
full text editing (cursor movement, selection, paste, IME).

## Approach

Official `gpui` 0.2.2 only. Text entry is a minimal hand-rolled field: a focusable
`div` with `on_key_down` handling printable characters, Backspace, Enter (add),
Escape (clear). Rejected: `gpui-component` (depends on forked `gpui-pre`, not the
official crate); porting Zed's full `input` example (~600 lines, overkill here).

Build note: `gpui` is used with the `runtime_shaders` feature. Newer Xcode ships
the `metal` compiler as a separate "Metal Toolchain" download; with this feature
GPUI compiles its shaders at launch via the Metal framework instead.

## Architecture

- `examples/hello.rs` — the gpui.rs hello world, to verify the toolchain.
- `src/board.rs` — pure model, no UI:
  - `Status { Todo, Doing, Done }`
  - `Card { id: u64, title: String, status: Status }`
  - `Board { cards: Vec<Card>, next_id: u64 }` with `add`, `delete`, `move_to`
    (changes status and moves the card to the end of the list so it lands at the
    bottom of its new column), `cards_in(status)`, `sample()`.
  - `load(path)` / `save(path)` via serde_json.
- `src/main.rs` — GPUI app:
  - `KanbanBoard` view entity owning the `Board`, the save path, and per-column
    draft text + `FocusHandle`.
  - Cards are draggable (`on_drag` with a `DraggedCard { id, title }` payload and
    a small preview view). Columns are drop targets (`on_drop`, `drag_over`
    highlight).
  - Cmd-Q quits; closing the window quits.

## Data flow

User action → mutate `Board` → `save()` → `cx.notify()` to re-render.

## Persistence & errors

- File: `~/Library/Application Support/gpui-kanban/board.json`.
- Missing file → sample board.
- Unparseable file → renamed to `board.json.bak`, start with sample board.
- Save failure → logged to stderr; app keeps running.

## Testing

- Unit tests in `board.rs`: add, delete, move, `cards_in` ordering, JSON
  round-trip, load of missing/corrupt file.
- Manual: `cargo run`, plus a screenshot to confirm it renders.
