//! Property-based ("fuzz") tests for the board.
//!
//! Proptest generates hundreds of random operation sequences and files per
//! test, and shrinks any failure down to a minimal example. For a longer run:
//! `PROPTEST_CASES=10000 cargo test --release fuzz`.

use super::*;
use proptest::prelude::*;
use proptest::sample::Index;
use std::collections::HashSet;

fn status() -> impl Strategy<Value = Status> {
    prop_oneof![Just(Status::Todo), Just(Status::Doing), Just(Status::Done)]
}

/// Titles that are hard on text handling.
fn title() -> impl Strategy<Value = String> {
    let nasties: Vec<String> = vec![
        "👩🏽‍💻".into(),            // emoji joined with zero-width joiners
        "مرحبا بالعالم".into(), // right-to-left
        "e\u{301}".into(),      // e + combining acute accent
        "\u{200b}".into(),      // zero-width space (not whitespace to Unicode)
        "\0".into(),            // NUL
        "\"quoted\" \\ back\\slash".into(),
        "🦀".repeat(500),
    ];
    prop_oneof![
        4 => "\\PC{1,40}",                                // any printable Unicode
        1 => "[ \t\n\u{a0}\u{2003}]{0,6}",                // whitespace only: must be rejected
        1 => "[ \t]{0,3}[a-z]{1,10}[ \t\n]{0,3}",         // padded: must be trimmed
        1 => prop::collection::vec(any::<char>(), 0..3000)
            .prop_map(String::from_iter),                // huge, any char at all
        1 => prop::sample::select(nasties),
    ]
}

/// Which card an operation is aimed at.
#[derive(Debug, Clone)]
enum Target {
    /// One of the cards currently on the board.
    Existing(Index),
    /// Any id; it may exist, may have been deleted, or may never have existed.
    AnyId(u64),
}

fn target() -> impl Strategy<Value = Target> {
    prop_oneof![
        3 => any::<Index>().prop_map(Target::Existing),
        1 => (0u64..300).prop_map(Target::AnyId),
    ]
}

impl Target {
    fn resolve(&self, board: &Board) -> u64 {
        match self {
            Target::Existing(index) if !board.cards.is_empty() => {
                board.cards[index.index(board.cards.len())].id
            }
            Target::Existing(_) => 0,
            Target::AnyId(id) => *id,
        }
    }
}

/// Ids as a hand-edited or damaged file might have them: small numbers that
/// collide, the largest possible values, or anything at all.
fn file_id() -> impl Strategy<Value = u64> {
    prop_oneof![
        2 => 0u64..8,
        1 => Just(u64::MAX),
        1 => Just(u64::MAX - 1),
        1 => any::<u64>(),
    ]
}

#[derive(Debug, Clone)]
enum Op {
    Add(String, Status),
    Delete(Target),
    Move(Target, Status),
    SaveAndLoad,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (title(), status()).prop_map(|(title, status)| Op::Add(title, status)),
        2 => target().prop_map(Op::Delete),
        3 => (target(), status()).prop_map(|(target, status)| Op::Move(target, status)),
        1 => Just(Op::SaveAndLoad),
    ]
}

/// The board kept as three separate lists, one per column: the most obvious
/// way to model a kanban board, and deliberately not how `Board` stores it.
#[derive(Debug, Default)]
struct Model {
    columns: [Vec<(u64, String)>; 3],
    next_id: u64,
}

impl Model {
    fn add(&mut self, title: &str, status: Status) -> Option<u64> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.columns[status as usize].push((id, title.to_string()));
        Some(id)
    }

    fn find(&self, id: u64) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(column, cards)| {
            let row = cards.iter().position(|(card_id, _)| *card_id == id)?;
            Some((column, row))
        })
    }

    fn delete(&mut self, id: u64) -> bool {
        let Some((column, row)) = self.find(id) else {
            return false;
        };
        self.columns[column].remove(row);
        true
    }

    fn move_to(&mut self, id: u64, status: Status) -> bool {
        let Some((column, row)) = self.find(id) else {
            return false;
        };
        if column == status as usize {
            return false;
        }
        let card = self.columns[column].remove(row);
        self.columns[status as usize].push(card);
        true
    }
}

/// What must hold for any board, however it was made or loaded.
fn check_structure(board: &Board) -> Result<(), TestCaseError> {
    let ids: HashSet<u64> = board.cards.iter().map(|card| card.id).collect();
    prop_assert_eq!(ids.len(), board.cards.len(), "duplicate ids");
    prop_assert!(
        board.cards.iter().all(|card| card.id < board.next_id),
        "next_id {} is not above every id",
        board.next_id
    );

    let mut columns = Default::default();
    board.index_columns(&mut columns);
    let mut seen = vec![false; board.cards.len()];
    for (status, column) in Status::ALL.into_iter().zip(&columns) {
        for &position in column {
            prop_assert_eq!(board.cards[position].status, status);
            prop_assert!(!seen[position], "card at {} listed twice", position);
            seen[position] = true;
        }
    }
    prop_assert!(
        seen.iter().all(|&s| s),
        "a card is missing from the columns"
    );
    Ok(())
}

/// What must also hold for a board built only through `add`.
fn check_titles(board: &Board) -> Result<(), TestCaseError> {
    for card in &board.cards {
        prop_assert!(!card.title.is_empty(), "empty title");
        prop_assert_eq!(card.title.trim(), card.title.as_str(), "untrimmed title");
    }
    Ok(())
}

fn check_matches_model(board: &Board, model: &Model) -> Result<(), TestCaseError> {
    for (status, column) in Status::ALL.into_iter().zip(&model.columns) {
        let actual: Vec<(u64, &str)> = board
            .cards_in(status)
            .map(|card| (card.id, card.title.as_str()))
            .collect();
        let expected: Vec<(u64, &str)> = column
            .iter()
            .map(|(id, title)| (*id, title.as_str()))
            .collect();
        prop_assert_eq!(actual, expected, "column {:?}", status);
    }
    prop_assert_eq!(board.next_id, model.next_id);
    Ok(())
}

/// Loading must never lose a file it can't use: after loading and then saving
/// (which the app does on the next change), unparseable bytes must survive in
/// `.bak`. Whatever `load` returns must also be a sound, usable board.
fn check_load_keeps(bytes: &[u8]) -> Result<(), TestCaseError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.json");
    fs::write(&path, bytes).unwrap();

    let mut board = Board::load(&path);
    check_structure(&board)?;
    board.add("Still usable", Status::Todo);
    check_structure(&board)?;
    board.save(&path).unwrap();

    if serde_json::from_slice::<Board>(bytes).is_err() {
        let backup = fs::read(path.with_extension("json.bak")).ok();
        prop_assert!(
            backup.as_deref() == Some(bytes),
            "an unreadable file was not preserved in .bak"
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn random_operations_match_the_model(ops in prop::collection::vec(op(), 0..200)) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut board = Board::default();
        let mut model = Model::default();

        for op in ops {
            match op {
                Op::Add(title, status) => {
                    prop_assert_eq!(board.add(&title, status), model.add(&title, status));
                }
                Op::Delete(target) => {
                    let id = target.resolve(&board);
                    prop_assert_eq!(board.delete(id), model.delete(id));
                }
                Op::Move(target, status) => {
                    let id = target.resolve(&board);
                    prop_assert_eq!(board.move_to(id, status), model.move_to(id, status));
                }
                Op::SaveAndLoad => {
                    board.save(&path).unwrap();
                    let loaded = Board::load(&path);
                    prop_assert_eq!(&loaded, &board);
                    board = loaded;
                }
            }
            check_matches_model(&board, &model)?;
            check_structure(&board)?;
            check_titles(&board)?;
        }
    }

    #[test]
    fn loading_any_bytes_is_safe(bytes in prop::collection::vec(any::<u8>(), 0..2000)) {
        check_load_keeps(&bytes)?;
    }

    #[test]
    fn loading_a_truncated_save_is_safe(cards in 0usize..40, seed: u64, cut: Index) {
        let json = serde_json::to_vec_pretty(&Board::stress(cards, seed)).unwrap();
        check_load_keeps(&json[..cut.index(json.len())])?;
    }

    #[test]
    fn loading_a_corrupted_save_is_safe(
        cards in 1usize..40,
        seed: u64,
        damage in prop::collection::vec((any::<Index>(), b'0'..=b'9'), 1..4),
    ) {
        let mut json = serde_json::to_vec_pretty(&Board::stress(cards, seed)).unwrap();
        for (at, digit) in damage {
            let at = at.index(json.len());
            json[at] = digit;
        }
        check_load_keeps(&json)?;
    }

    #[test]
    fn loading_boards_with_any_ids_is_safe(
        ids in prop::collection::vec(file_id(), 0..20),
        next_id in file_id(),
    ) {
        let board = Board {
            cards: ids
                .into_iter()
                .map(|id| Card { id, title: format!("Card {id}"), status: Status::Todo })
                .collect(),
            next_id,
        };
        check_load_keeps(&serde_json::to_vec(&board).unwrap())?;
    }
}
