//! Frame timing for stress mode.
//!
//! In stress mode the window redraws every frame, so the gap between two
//! renders is the real frame time: building elements, layout, paint, and
//! waiting for the display. On a 120 Hz display a healthy gap is about 8.3 ms.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub struct Perf {
    /// (when the frame started, gap since the previous one) for the last second.
    frames: VecDeque<(Instant, Duration)>,
    last_frame: Option<Instant>,
    last_report: Instant,
    /// Time spent building the element tree in the last `render()`.
    pub build: Duration,
    /// How long the last background save took (off the UI thread).
    pub save: Duration,
}

impl Perf {
    pub fn new() -> Self {
        Self {
            frames: VecDeque::new(),
            last_frame: None,
            last_report: Instant::now(),
            build: Duration::ZERO,
            save: Duration::ZERO,
        }
    }

    /// Call at the start of every render.
    pub fn frame_started(&mut self) {
        let now = Instant::now();
        if let Some(previous) = self.last_frame.replace(now) {
            self.frames.push_back((now, now - previous));
        }
        while let Some(&(at, _)) = self.frames.front() {
            if now - at <= Duration::from_secs(1) {
                break;
            }
            self.frames.pop_front();
        }
    }

    pub fn summary(&self, cards: usize, chaos: bool) -> String {
        let count = self.frames.len();
        let total: Duration = self.frames.iter().map(|(_, gap)| *gap).sum();
        let average = if count == 0 {
            Duration::ZERO
        } else {
            total / count as u32
        };
        let worst = self.frames.iter().map(|(_, gap)| *gap).max();
        format!(
            "{cards} cards · {count} fps · avg {:.1} ms · worst {:.1} ms · build {:.1} ms · bg save {:.1} ms{}",
            ms(average),
            ms(worst.unwrap_or_default()),
            ms(self.build),
            ms(self.save),
            if chaos { " · chaos" } else { "" },
        )
    }

    /// Prints the summary at most once a second, for scripted benchmarks.
    pub fn maybe_report(&mut self, summary: &str) {
        if self.last_report.elapsed() >= Duration::from_secs(1) {
            self.last_report = Instant::now();
            println!("[perf] {summary}");
        }
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
