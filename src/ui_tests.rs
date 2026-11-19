//! Headless UI tests: the real `KanbanBoard` view, driven by simulated mouse
//! and keyboard input through GPUI's test harness. No window is shown, and
//! time only moves when a test advances the fake clock.

use super::*;
use gpui::{
    Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point,
};
use tempfile::TempDir;

/// Opens a board in a test window. It saves to a fresh temp folder, which
/// lives as long as the returned `TempDir`.
fn open(
    cx: &mut TestAppContext,
    board: Board,
) -> (
    Entity<KanbanBoard>,
    &mut VisualTestContext,
    TempDir,
    PathBuf,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.json");
    let save_path = path.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| KanbanBoard::new(board, save_path, window, cx));
    cx.run_until_parked();
    (view, cx, dir, path)
}

/// The centre of the element with this `debug_selector`, which must be on screen.
fn center(cx: &mut VisualTestContext, selector: String) -> Point<Pixels> {
    // `debug_bounds` wants a `&'static str`; leaking a few bytes is fine in a test.
    let selector: &'static str = Box::leak(selector.into_boxed_str());
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"))
        .center()
}

fn click(cx: &mut VisualTestContext, selector: String) {
    let at = center(cx, selector);
    cx.simulate_click(at, Modifiers::none());
}

/// Mouse down on `from`, move far enough to start a drag, then release over `to`.
fn drag(cx: &mut VisualTestContext, from: String, to: String) {
    let start = center(cx, from);
    let end = center(cx, to);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        start + point(px(10.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
}

fn board(view: &Entity<KanbanBoard>, cx: &mut VisualTestContext) -> Board {
    view.read_with(cx, |view, _| view.board.clone())
}

fn titles(board: &Board, status: Status) -> Vec<String> {
    board
        .cards_in(status)
        .map(|card| card.title.clone())
        .collect()
}

fn id_of(board: &Board, title: &str) -> u64 {
    board
        .cards
        .iter()
        .find(|card| card.title == title)
        .unwrap_or_else(|| panic!("no card called {title:?}"))
        .id
}

fn small_board() -> Board {
    let mut board = Board::default();
    board.add("A", Status::Todo);
    board.add("B", Status::Todo);
    board.add("C", Status::Doing);
    board
}

// ---- Typing ----

#[gpui::test]
fn typing_a_title_and_pressing_enter_adds_a_card(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());

    click(cx, "input-1".into());
    cx.simulate_keystrokes("H i space t h e r e enter");

    assert_eq!(titles(&board(&view, cx), Status::Doing), ["C", "Hi there"]);
    assert_eq!(view.read_with(cx, |view, _| view.drafts[1].clone()), "");
}

#[gpui::test]
fn backspace_and_escape_edit_the_draft(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());

    click(cx, "input-0".into());
    cx.simulate_keystrokes("a b c backspace");
    assert_eq!(view.read_with(cx, |view, _| view.drafts[0].clone()), "ab");

    cx.simulate_keystrokes("escape");
    assert_eq!(view.read_with(cx, |view, _| view.drafts[0].clone()), "");
    let focused = cx.update(|window, cx| view.read(cx).input_focus[0].is_focused(window));
    assert!(!focused, "Escape should leave the field");
    assert_eq!(board(&view, cx), small_board(), "nothing was added");
}

#[gpui::test]
fn enter_on_a_blank_draft_adds_nothing(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());

    click(cx, "input-2".into());
    cx.simulate_keystrokes("space space enter");

    assert_eq!(board(&view, cx), small_board());
}

#[gpui::test]
fn shortcuts_are_not_typed_into_the_field(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());

    click(cx, "input-0".into());
    cx.simulate_keystrokes("cmd-c ctrl-a x");

    assert_eq!(view.read_with(cx, |view, _| view.drafts[0].clone()), "x");
}

// ---- Deleting and dragging ----

#[gpui::test]
fn clicking_the_x_deletes_that_card(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());
    let b = id_of(&board(&view, cx), "B");

    click(cx, format!("delete-{b}"));

    let after = board(&view, cx);
    assert_eq!(titles(&after, Status::Todo), ["A"]);
    assert_eq!(titles(&after, Status::Doing), ["C"]);
}

#[gpui::test]
fn dragging_a_card_to_another_column_moves_it_to_the_bottom(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());
    let a = id_of(&board(&view, cx), "A");

    drag(cx, format!("card-{a}"), "column-1".into());

    let after = board(&view, cx);
    assert_eq!(titles(&after, Status::Todo), ["B"]);
    assert_eq!(titles(&after, Status::Doing), ["C", "A"]);
}

#[gpui::test]
fn dropping_a_card_on_its_own_column_changes_nothing(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, small_board());
    let a = id_of(&board(&view, cx), "A");

    drag(cx, format!("card-{a}"), "column-0".into());

    assert_eq!(board(&view, cx), small_board());
}

// ---- Saving ----

#[gpui::test]
fn changes_are_saved_after_a_short_delay(cx: &mut TestAppContext) {
    let (view, cx, _dir, path) = open(cx, small_board());

    click(cx, "input-0".into());
    cx.simulate_keystrokes("n e w enter");
    assert!(
        !path.exists(),
        "nothing should be written before SAVE_INTERVAL"
    );

    cx.executor().advance_clock(SAVE_INTERVAL);
    cx.run_until_parked();
    assert_eq!(Board::load(&path), board(&view, cx));
}

#[gpui::test]
fn quick_changes_are_folded_into_one_save(cx: &mut TestAppContext) {
    let (view, cx, _dir, path) = open(cx, small_board());

    click(cx, "input-0".into());
    for _ in 0..5 {
        cx.simulate_keystrokes("x enter");
    }
    cx.executor().advance_clock(SAVE_INTERVAL);
    cx.run_until_parked();

    assert_eq!(
        view.read_with(cx, |view, _| view.saver.snapshots_taken()),
        1
    );
    assert_eq!(titles(&Board::load(&path), Status::Todo).len(), 2 + 5);
}

#[gpui::test]
fn quitting_saves_pending_changes_immediately(app: &mut TestAppContext) {
    let (view, cx, _dir, path) = open(app, small_board());

    click(cx, "input-0".into());
    cx.simulate_keystrokes("l a s t enter");
    let expected = board(&view, cx);
    assert!(!path.exists());

    // Runs the app's quit hooks, like Cmd-Q. This must be called on the
    // TestAppContext, not the window's VisualTestContext: in gpui 0.2.2,
    // quitting frees the VisualTestContext (see `into_mut`), so calling
    // `quit()` through it is a use-after-free that crashes with SIGSEGV.
    app.quit();

    assert_eq!(Board::load(&path), expected);
}

#[gpui::test]
fn closing_the_window_saves_pending_changes_immediately(cx: &mut TestAppContext) {
    let (view, cx, _dir, path) = open(cx, small_board());

    click(cx, "input-0".into());
    cx.simulate_keystrokes("l a s t enter");
    let expected = board(&view, cx);
    assert!(!path.exists());

    drop(view); // the window should be the last thing holding the view
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();

    assert_eq!(Board::load(&path), expected);
}

// ---- Soak ----

/// How many soak steps to run: `SOAK_STEPS` if set, otherwise 1,000 in
/// release builds (~5 s) and 100 in debug builds, where rendering is about
/// ten times slower.
fn soak_steps() -> usize {
    std::env::var("SOAK_STEPS")
        .ok()
        .and_then(|steps| steps.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 100 } else { 1000 })
}

/// Random drags, deletes and adds through the UI on a 10,000-card board.
/// After every step the view must agree with a board that had the same
/// operations applied directly. `cargo test --release` runs 1,000 steps.
#[gpui::test]
fn soak_random_ui_operations_on_10k_cards(cx: &mut TestAppContext) {
    let (view, cx, _dir, _) = open(cx, Board::stress(10_000, 42));
    let mut expected = board(&view, cx);
    let mut rng = fastrand::Rng::with_seed(7);
    let mut columns = Default::default();

    for step in 0..soak_steps() {
        expected.index_columns(&mut columns);
        let from = rng.usize(..3);
        if columns[from].is_empty() {
            continue;
        }
        // Only the top rows of each column are on screen (the lists are virtualized).
        let row = rng.usize(..columns[from].len().min(5));
        let id = expected.cards[columns[from][row]].id;
        let to = rng.usize(..3);

        match rng.u8(..10) {
            0 => {
                click(cx, format!("delete-{id}"));
                expected.delete(id);
            }
            1 => {
                click(cx, format!("input-{to}"));
                cx.simulate_keystrokes("s o a k enter");
                expected.add("soak", Status::ALL[to]);
            }
            _ => {
                drag(cx, format!("card-{id}"), format!("column-{to}"));
                expected.move_to(id, Status::ALL[to]);
            }
        }

        let matches = view.read_with(cx, |view, _| view.board == expected);
        assert!(
            matches,
            "the UI and the expected board differ after step {step}"
        );
    }
}
