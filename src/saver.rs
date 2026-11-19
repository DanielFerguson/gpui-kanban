//! Writes the board to disk off the UI thread.
//!
//! The UI thread takes a numbered snapshot (a clone: about 2 ms for 100k
//! cards) and a background thread does the slow part (JSON and disk: about
//! 15 ms). Writes can finish out of order (say, a background save is still
//! running when the app saves one last time on quit), so an older snapshot
//! never overwrites a newer one.

use crate::board::Board;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct Saver {
    path: PathBuf,
    /// The number of the newest snapshot on disk. Holding the lock while
    /// writing also stops two writes from overlapping.
    written: Arc<Mutex<u64>>,
    taken: u64,
}

impl Saver {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            written: Arc::default(),
            taken: 0,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many snapshots have been taken; lets tests see saves being folded together.
    #[cfg(test)]
    pub fn snapshots_taken(&self) -> u64 {
        self.taken
    }

    /// Copies the board as it is now, ready to write from any thread.
    pub fn snapshot(&mut self, board: &Board) -> Snapshot {
        self.taken += 1;
        Snapshot {
            number: self.taken,
            board: board.clone(),
            path: self.path.clone(),
            written: self.written.clone(),
        }
    }
}

pub struct Snapshot {
    number: u64,
    board: Board,
    path: PathBuf,
    written: Arc<Mutex<u64>>,
}

impl Snapshot {
    /// Writes the snapshot unless a newer one is already on disk, and returns
    /// whether it did.
    pub fn write(self) -> io::Result<bool> {
        // A panic mid-write can't leave the number wrong, so a poisoned lock is fine to reuse.
        let mut written = self
            .written
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.number <= *written {
            return Ok(false);
        }
        self.board.save(&self.path)?;
        *written = self.number;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Status;
    use proptest::prelude::*;

    /// Snapshots of a board that gains one card per snapshot.
    fn growing_snapshots(saver: &mut Saver, count: usize) -> (Board, Vec<Snapshot>) {
        let mut board = Board::default();
        let snapshots = (0..count)
            .map(|i| {
                board.add(&format!("Card {i}"), Status::Todo);
                saver.snapshot(&board)
            })
            .collect();
        (board, snapshots)
    }

    proptest! {
        #[test]
        fn snapshots_written_in_any_order_leave_the_newest_on_disk(
            order in (1usize..8).prop_flat_map(|n| Just((0..n).collect::<Vec<_>>()).prop_shuffle())
        ) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("board.json");
            let mut saver = Saver::new(path.clone());
            let (newest, snapshots) = growing_snapshots(&mut saver, order.len());
            let mut snapshots: Vec<Option<Snapshot>> = snapshots.into_iter().map(Some).collect();

            let mut newest_written: Option<usize> = None;
            for i in order {
                let written = snapshots[i].take().unwrap().write().unwrap();
                // A snapshot is written only if it's newer than everything written so far.
                prop_assert_eq!(written, newest_written.is_none_or(|n| i > n));
                if written {
                    newest_written = Some(i);
                }
            }
            prop_assert_eq!(Board::load(&path), newest);
        }

        #[test]
        fn snapshots_written_at_the_same_time_leave_the_newest_on_disk(count in 1usize..8) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("board.json");
            let mut saver = Saver::new(path.clone());
            let (newest, snapshots) = growing_snapshots(&mut saver, count);

            let threads: Vec<_> = snapshots
                .into_iter()
                .map(|snapshot| std::thread::spawn(move || snapshot.write().unwrap()))
                .collect();
            for thread in threads {
                thread.join().unwrap();
            }
            prop_assert_eq!(Board::load(&path), newest);
        }
    }

    #[test]
    fn a_snapshot_saves_the_board_as_it_was_when_taken() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut saver = Saver::new(path.clone());
        let mut board = Board::sample();
        let before = board.clone();

        let snapshot = saver.snapshot(&board);
        board.add("Added after the snapshot", Status::Todo);

        assert!(snapshot.write().unwrap());
        assert_eq!(Board::load(&path), before);
    }

    #[test]
    fn an_older_snapshot_never_overwrites_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut saver = Saver::new(path.clone());
        let mut board = Board::sample();
        let older = saver.snapshot(&board);
        board.add("Newer", Status::Doing);
        let newer = saver.snapshot(&board);

        assert!(newer.write().unwrap());
        assert!(
            !older.write().unwrap(),
            "the older snapshot should be skipped"
        );
        assert_eq!(Board::load(&path), board);
    }

    #[test]
    fn snapshots_can_be_written_from_another_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.json");
        let mut saver = Saver::new(path.clone());
        let snapshot = saver.snapshot(&Board::stress(1000, 1));

        let written = std::thread::spawn(move || snapshot.write()).join().unwrap();

        assert!(written.unwrap());
        assert_eq!(Board::load(&path), Board::stress(1000, 1));
    }
}
