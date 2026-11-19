//! The kanban board's data, with no UI code, so it can be unit-tested.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

/// The largest id or `next_id` accepted from a file: 2^53 - 1, the biggest
/// integer every JSON tool agrees on. Anything larger means the file was
/// damaged or hand-edited, and its cards get renumbered.
const MAX_FILE_ID: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    /// The columns, left to right.
    pub const ALL: [Status; 3] = [Status::Todo, Status::Doing, Status::Done];

    pub fn label(self) -> &'static str {
        match self {
            Status::Todo => "To Do",
            Status::Doing => "Doing",
            Status::Done => "Done",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: u64,
    pub title: String,
    pub status: Status,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    pub cards: Vec<Card>,
    pub next_id: u64,
}

impl Board {
    /// What a brand-new board starts with.
    pub fn sample() -> Self {
        let mut board = Board::default();
        board.add("Install Rust and Xcode", Status::Done);
        board.add("Run the GPUI hello world", Status::Done);
        board.add("Build a kanban board", Status::Doing);
        board.add("Drag this card to Doing", Status::Todo);
        board.add("Add a card of your own", Status::Todo);
        board
    }

    /// Adds a card and returns its id. Blank titles are ignored.
    pub fn add(&mut self, title: &str, status: Status) -> Option<u64> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.cards.push(Card {
            id,
            title: title.to_string(),
            status,
        });
        Some(id)
    }

    /// Deletes a card. Returns whether anything was removed.
    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.cards.len();
        self.cards.retain(|card| card.id != id);
        self.cards.len() != before
    }

    /// Moves a card to the bottom of another column. Returns whether anything changed.
    pub fn move_to(&mut self, id: u64, status: Status) -> bool {
        let Some(ix) = self.cards.iter().position(|card| card.id == id) else {
            return false;
        };
        if self.cards[ix].status == status {
            return false;
        }
        let mut card = self.cards.remove(ix);
        card.status = status;
        self.cards.push(card);
        true
    }

    /// The cards in one column, top to bottom. The UI uses `index_columns`
    /// instead; this simpler version is the reference the tests check against.
    #[cfg(test)]
    pub fn cards_in(&self, status: Status) -> impl Iterator<Item = &Card> {
        self.cards.iter().filter(move |card| card.status == status)
    }

    /// For each column (in `Status::ALL` order), the positions in `cards` of
    /// its cards, top to bottom. Reuses `out`'s allocations, since the UI
    /// calls this every frame.
    pub fn index_columns(&self, out: &mut [Vec<usize>; 3]) {
        out.iter_mut().for_each(Vec::clear);
        for (position, card) in self.cards.iter().enumerate() {
            // Statuses are declared in column order, so this is the column number.
            out[card.status as usize].push(position);
        }
    }

    /// Reads a saved board. Never fails: a missing or broken file gives the sample board.
    pub fn load(path: &Path) -> Self {
        // Read raw bytes and let the JSON parser reject bad UTF-8, so every
        // unusable file takes the same back-it-up path below.
        let problem = match fs::read(path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Board::sample(),
            Err(err) => format!("couldn't be read ({err})"),
            Ok(bytes) => match serde_json::from_slice::<Board>(&bytes) {
                Ok(mut board) => {
                    board.repair_ids();
                    return board;
                }
                Err(err) => format!("isn't a valid board ({err})"),
            },
        };
        // Move the file aside rather than letting the next save overwrite it.
        let backup = path.with_extension("json.bak");
        eprintln!(
            "{} {problem}. Moving it to {} and starting fresh.",
            path.display(),
            backup.display()
        );
        if let Err(err) = fs::rename(path, &backup) {
            eprintln!("Couldn't move it: {err}");
        }
        Board::sample()
    }

    /// Makes a loaded board's ids safe to use, in case the file was
    /// hand-edited or damaged: ids must be unique, and `next_id` must be above
    /// all of them with room to grow. Ids are never shown, so if they're
    /// unusable, the cards are simply renumbered.
    fn repair_ids(&mut self) {
        let mut seen = HashSet::new();
        let usable = self.next_id <= MAX_FILE_ID
            && self
                .cards
                .iter()
                .all(|card| card.id < MAX_FILE_ID && seen.insert(card.id));
        if usable {
            if let Some(max_id) = self.cards.iter().map(|card| card.id).max() {
                self.next_id = self.next_id.max(max_id + 1); // can't overflow: max_id < MAX_FILE_ID
            }
        } else {
            for (id, card) in (0..).zip(&mut self.cards) {
                card.id = id;
            }
            self.next_id = self.cards.len() as u64;
        }
    }

    /// Writes to a temporary file first, then renames it, so a crash mid-save
    /// can't leave a half-written board behind.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(&tmp, path)
    }

    /// A big, reproducible board for stress testing: `count` cards whose titles
    /// mix short, long, emoji, accented, CJK and right-to-left text.
    pub fn stress(count: usize, seed: u64) -> Self {
        const TITLES: &[&str] = &[
            "Fix login bug",
            "Write the release notes, including the migration guide, known issues and a thank-you to every contributor",
            "🚀 Ship it",
            "Café résumé naïve façade",
            "修复用户界面中的文字换行问题",
            "مراجعة طلب الدمج قبل الإصدار",
            "Refactor drag and drop so columns and cards share one hit-testing path",
            "👩🏽‍💻 Pair on the 🐛 with 🦀",
        ];
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut board = Board::default();
        for i in 0..count {
            let title = format!("#{i} {}", TITLES[rng.usize(..TITLES.len())]);
            board.add(&title, Status::ALL[rng.usize(..Status::ALL.len())]);
        }
        board
    }

    /// One frame of chaos: move `moves` random cards to random columns, and
    /// sometimes add or delete one, so the board keeps changing shape.
    pub fn chaos(&mut self, rng: &mut fastrand::Rng, moves: usize) {
        let random_status = |rng: &mut fastrand::Rng| Status::ALL[rng.usize(..Status::ALL.len())];
        for _ in 0..moves {
            if self.cards.is_empty() {
                break;
            }
            let id = self.cards[rng.usize(..self.cards.len())].id;
            let status = random_status(rng);
            self.move_to(id, status);
        }
        if rng.bool() {
            let title = format!("Chaos card {}", self.next_id);
            let status = random_status(rng);
            self.add(&title, status);
        }
        if rng.bool() && !self.cards.is_empty() {
            let id = self.cards[rng.usize(..self.cards.len())].id;
            self.delete(id);
        }
    }
}

/// `~/Library/Application Support/gpui-kanban/board.json` on macOS.
pub fn default_path() -> PathBuf {
    data_file("board.json")
}

/// A file in `~/Library/Application Support/gpui-kanban/` on macOS.
pub fn data_file(name: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gpui-kanban")
        .join(name)
}

#[cfg(test)]
mod fuzz;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn titles(board: &Board, status: Status) -> Vec<&str> {
        board.cards_in(status).map(|c| c.title.as_str()).collect()
    }

    /// Ids are unique and `next_id` is above all of them.
    fn assert_ids_consistent(board: &Board) {
        let ids: HashSet<u64> = board.cards.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), board.cards.len(), "duplicate ids");
        assert!(ids.iter().all(|&id| id < board.next_id), "next_id too low");
    }

    #[test]
    fn default_path_is_in_the_app_data_folder() {
        let path = default_path();
        assert!(path.ends_with("gpui-kanban/board.json"), "{path:?}");
        assert_eq!(data_file("stress.json").parent(), path.parent());
    }

    #[test]
    fn stress_board_is_deterministic_and_well_formed() {
        let board = Board::stress(1000, 7);
        assert_eq!(board, Board::stress(1000, 7));
        assert_ne!(board, Board::stress(1000, 8));
        assert_eq!(board.cards.len(), 1000);
        assert_eq!(board.next_id, 1000);
        assert_ids_consistent(&board);
        for status in Status::ALL {
            assert!(board.cards_in(status).count() > 0, "{status:?} is empty");
        }
    }

    #[test]
    fn index_columns_matches_cards_in() {
        let board = Board::stress(500, 9);
        let mut columns = [vec![999], vec![], vec![]]; // stale data must be cleared
        board.index_columns(&mut columns);
        for (status, column) in Status::ALL.into_iter().zip(&columns) {
            let via_index: Vec<&Card> = column.iter().map(|&i| &board.cards[i]).collect();
            let via_filter: Vec<&Card> = board.cards_in(status).collect();
            assert_eq!(via_index, via_filter, "{status:?}");
        }
    }

    #[test]
    fn chaos_keeps_the_board_consistent() {
        let mut rng = fastrand::Rng::with_seed(3);
        let mut board = Board::stress(100, 1);
        for _ in 0..1000 {
            board.chaos(&mut rng, 20);
            assert_ids_consistent(&board);
        }
        // Chaos adds and deletes at the same rate, so the size should stay sane.
        assert!(
            (20..=300).contains(&board.cards.len()),
            "{}",
            board.cards.len()
        );

        let mut empty = Board::default();
        for _ in 0..100 {
            empty.chaos(&mut rng, 20);
            assert_ids_consistent(&empty);
        }
    }

    #[test]
    fn add_gives_each_card_a_new_id() {
        let mut board = Board::default();
        assert_eq!(board.add("First", Status::Todo), Some(0));
        assert_eq!(board.add("Second", Status::Done), Some(1));
        assert_eq!(board.cards.len(), 2);
        assert_eq!(board.cards[1].status, Status::Done);
    }

    #[test]
    fn add_trims_titles_and_ignores_blank_ones() {
        let mut board = Board::default();
        assert_eq!(board.add("   ", Status::Todo), None);
        board.add("  Buy milk  ", Status::Todo).unwrap();
        assert_eq!(titles(&board, Status::Todo), ["Buy milk"]);
    }

    #[test]
    fn delete_removes_only_that_card() {
        let mut board = Board::default();
        let a = board.add("A", Status::Todo).unwrap();
        board.add("B", Status::Todo).unwrap();
        assert!(board.delete(a));
        assert!(!board.delete(a));
        assert_eq!(titles(&board, Status::Todo), ["B"]);
    }

    #[test]
    fn move_to_changes_status_and_puts_card_at_bottom_of_new_column() {
        let mut board = Board::default();
        let a = board.add("A", Status::Todo).unwrap();
        board.add("B", Status::Done).unwrap();
        assert!(board.move_to(a, Status::Done));
        assert_eq!(titles(&board, Status::Done), ["B", "A"]);
        assert!(titles(&board, Status::Todo).is_empty());
    }

    #[test]
    fn move_to_same_column_or_unknown_card_does_nothing() {
        let mut board = Board::default();
        let a = board.add("A", Status::Todo).unwrap();
        board.add("B", Status::Todo).unwrap();
        let before = board.clone();
        assert!(!board.move_to(a, Status::Todo));
        assert!(!board.move_to(999, Status::Done));
        assert_eq!(board, before);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-yet-created").join("board.json");
        let mut board = Board::sample();
        board.add("Extra", Status::Doing);
        board.save(&path).unwrap();
        assert_eq!(Board::load(&path), board);
    }

    #[test]
    fn load_missing_file_gives_sample_board() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Board::load(&dir.path().join("nope.json")), Board::sample());
    }

    #[test]
    fn load_corrupt_file_backs_it_up_and_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        fs::write(&path, "not json").unwrap();
        assert_eq!(Board::load(&path), Board::sample());
        assert!(!path.exists());
        let backup = dir.path().join("board.json.bak");
        assert_eq!(fs::read_to_string(backup).unwrap(), "not json");
    }

    /// Found by fuzzing: a file that isn't valid UTF-8 (say, cut off in the
    /// middle of an emoji) took the "couldn't read" path, which didn't back it
    /// up, so the next save overwrote it.
    #[test]
    fn load_invalid_utf8_backs_it_up_and_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let bytes = b"{\"cards\":[{\"id\":0,\"title\":\"\xF0\x9F\xA6";
        fs::write(&path, bytes).unwrap();
        assert_eq!(Board::load(&path), Board::sample());
        assert!(!path.exists());
        assert_eq!(fs::read(dir.path().join("board.json.bak")).unwrap(), bytes);
    }

    fn load_json(json: &str) -> Board {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        fs::write(&path, json).unwrap();
        Board::load(&path)
    }

    /// Found by fuzzing: with two cards sharing an id, deleting one deleted both.
    #[test]
    fn load_renumbers_duplicate_ids() {
        let mut board = load_json(
            r#"{"cards":[{"id":3,"title":"A","status":"Todo"},{"id":3,"title":"B","status":"Done"}],"next_id":4}"#,
        );
        assert_ids_consistent(&board);
        assert_eq!(titles(&board, Status::Todo), ["A"]);
        assert_eq!(titles(&board, Status::Done), ["B"]);
        let a = board.cards[0].id;
        assert!(board.delete(a));
        assert_eq!(
            titles(&board, Status::Done),
            ["B"],
            "deleting A must not delete B"
        );
    }

    /// Found by fuzzing: an id or next_id of u64::MAX overflowed when adding 1,
    /// crashing on launch (or on the next new card).
    #[test]
    fn load_survives_the_largest_possible_ids() {
        let mut board = load_json(&format!(
            r#"{{"cards":[{{"id":{max},"title":"A","status":"Todo"}}],"next_id":{max}}}"#,
            max = u64::MAX
        ));
        assert_ids_consistent(&board);
        assert!(board.add("B", Status::Todo).is_some());
        assert_ids_consistent(&board);
        assert_eq!(titles(&board, Status::Todo), ["A", "B"]);
    }

    #[test]
    fn load_repairs_next_id_that_would_reuse_an_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let json = r#"{"cards":[{"id":7,"title":"A","status":"Todo"}],"next_id":0}"#;
        fs::write(&path, json).unwrap();
        let mut board = Board::load(&path);
        assert_eq!(board.add("B", Status::Todo), Some(8));
    }
}
