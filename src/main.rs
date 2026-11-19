//! A tiny kanban board built with GPUI (https://gpui.rs). Run with `cargo run`.

mod board;
mod perf;
mod saver;
#[cfg(test)]
mod ui_tests;

use std::{
    ops::Range,
    path::PathBuf,
    process,
    time::{Duration, Instant},
};

use board::{Board, Card, Status};
use gpui::{
    App, Application, Bounds, Context, Div, FocusHandle, FontWeight, KeyBinding, KeyDownEvent,
    Menu, MenuItem, SharedString, Stateful, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size, uniform_list,
};
use perf::Perf;
use saver::Saver;

actions!(kanban, [Quit]);

const USAGE: &str = "usage: gpui-kanban [--stress N [--chaos]] [--exit-after SECS]";

/// Command-line options. With none, the app opens your normal board.
#[derive(Debug, Default, PartialEq)]
struct Options {
    /// `--stress N`: open a generated board of N cards, saved to stress.json.
    stress: Option<usize>,
    /// `--chaos`: start chaos mode straight away (stress mode only).
    chaos: bool,
    /// `--exit-after SECS`: quit after this long, for scripted benchmarks.
    exit_after: Option<Duration>,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Options::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--stress" => options.stress = Some(number(&mut args, "--stress")?),
                "--chaos" => options.chaos = true,
                "--exit-after" => {
                    options.exit_after =
                        Some(Duration::from_secs(number(&mut args, "--exit-after")?))
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }
        if options.chaos && options.stress.is_none() {
            // Chaos would scramble your real board.
            return Err("--chaos only works with --stress N".into());
        }
        Ok(options)
    }
}

fn number<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, String> {
    args.next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("{flag} needs a whole number"))
}

// A small dark palette.
const BACKGROUND: u32 = 0x1e1f24;
const COLUMN: u32 = 0x2a2c33;
const COLUMN_DROP: u32 = 0x313849;
const CARD: u32 = 0x3a3d47;
const CARD_HOVER: u32 = 0x444855;
const BORDER: u32 = 0x4a4e5a;
const TEXT: u32 = 0xe8e9ed;
const MUTED: u32 = 0x9499a6;
const ACCENT: u32 = 0x6c9cff;
const DANGER: u32 = 0xff6b6b;

/// How many cards chaos mode moves each frame.
const CHAOS_MOVES: usize = 20;

/// Changes are saved at most this often; rapid changes are folded together.
const SAVE_INTERVAL: Duration = Duration::from_millis(250);

const COLUMN_WIDTH: f32 = 300.;
/// A card fills its column minus the column's 12px padding on each side.
const CARD_WIDTH: f32 = COLUMN_WIDTH - 24.;

/// What travels with the mouse during a drag. GPUI also renders it as the
/// floating preview under the cursor.
#[derive(Clone)]
struct DraggedCard {
    id: u64,
    title: SharedString,
}

impl Render for DraggedCard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        card_shell()
            .w(px(CARD_WIDTH))
            .border_color(rgb(ACCENT))
            .shadow_lg()
            .child(self.title.clone())
    }
}

/// Shared look for a card and its drag preview.
fn card_shell() -> Div {
    div()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(CARD))
        .text_sm()
        .text_color(rgb(TEXT))
}

struct KanbanBoard {
    board: Board,
    saver: Saver,
    /// The board has changed since the last snapshot was taken.
    unsaved: bool,
    /// The background save loop is running.
    saving: bool,
    _flush_on_quit: Subscription,
    _flush_on_close: Subscription,
    /// Focused when no text field is, so Escape has somewhere to send focus.
    focus_handle: FocusHandle,
    /// The text being typed into each column's "add a card" field.
    drafts: [String; 3],
    input_focus: [FocusHandle; 3],
    /// Positions in `board.cards` of each column's cards, rebuilt every render.
    /// The virtualized card lists read it while GPUI lays them out.
    columns: [Vec<usize>; 3],
    /// Stress mode only: frame timing, shown in the header.
    perf: Option<Perf>,
    /// Stress mode only: scramble the board every frame.
    chaos: bool,
    rng: fastrand::Rng,
}

impl KanbanBoard {
    fn new(board: Board, save_path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        // Saves are delayed slightly, so make sure the last changes reach the
        // disk on Cmd-Q (app quit) and when the window closes (view released).
        let _flush_on_quit = cx.on_app_quit(|this, _| {
            this.flush();
            async {}
        });
        let _flush_on_close = cx.on_release(|this, _| this.flush());
        Self {
            board,
            saver: Saver::new(save_path),
            unsaved: false,
            saving: false,
            _flush_on_quit,
            _flush_on_close,
            focus_handle,
            drafts: Default::default(),
            input_focus: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            columns: Default::default(),
            perf: None,
            chaos: false,
            rng: fastrand::Rng::with_seed(42),
        }
    }

    /// Call after every change to the board: schedules a save and redraws.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.mark_unsaved(cx);
        cx.notify();
    }

    /// Saves at most once per `SAVE_INTERVAL`. The UI thread only takes a
    /// snapshot; the JSON and disk work happen on a background thread.
    fn mark_unsaved(&mut self, cx: &mut Context<Self>) {
        self.unsaved = true;
        if self.saving {
            return; // the running save loop will pick this change up
        }
        self.saving = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SAVE_INTERVAL).await;
                let snapshot = this.update(cx, |this, _| {
                    if !this.unsaved {
                        this.saving = false;
                        return None;
                    }
                    this.unsaved = false;
                    Some(this.saver.snapshot(&this.board))
                });
                let Ok(Some(snapshot)) = snapshot else {
                    break; // nothing left to save, or the view is gone
                };
                let started = Instant::now();
                let result = cx.background_spawn(async move { snapshot.write() }).await;
                let elapsed = started.elapsed();
                let still_open = this.update(cx, |this, _| {
                    this.report_save_error(result);
                    if let Some(perf) = &mut this.perf {
                        perf.save = elapsed;
                    }
                });
                if still_open.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Saves immediately on this thread, if anything is pending.
    fn flush(&mut self) {
        if !self.unsaved && !self.saving {
            return;
        }
        self.unsaved = false;
        let result = self.saver.snapshot(&self.board).write();
        self.report_save_error(result);
    }

    fn report_save_error(&self, result: std::io::Result<bool>) {
        if let Err(err) = result {
            eprintln!(
                "Couldn't save the board to {}: {err}",
                self.saver.path().display()
            );
        }
    }

    /// A minimal text field: typing, Backspace, Enter to add, Escape to cancel.
    fn on_input_key(
        &mut self,
        column: usize,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "enter" => {
                if self
                    .board
                    .add(&self.drafts[column], Status::ALL[column])
                    .is_some()
                {
                    self.drafts[column].clear();
                    self.changed(cx);
                }
            }
            "escape" => {
                self.drafts[column].clear();
                self.focus_handle.focus(window);
                cx.notify();
            }
            "backspace" => {
                self.drafts[column].pop();
                cx.notify();
            }
            _ => {
                let typed = keystroke
                    .key_char
                    .as_deref()
                    .filter(|text| !text.chars().any(char::is_control));
                match typed {
                    Some(text) if !keystroke.modifiers.platform && !keystroke.modifiers.control => {
                        self.drafts[column].push_str(text);
                        cx.notify();
                    }
                    // Not ours (e.g. cmd-c): let it keep bubbling.
                    _ => return,
                }
            }
        }
        cx.stop_propagation();
    }

    fn render_column(
        &self,
        ix: usize,
        status: Status,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let count = self.columns[ix].len();

        div()
            .debug_selector(|| format!("column-{ix}"))
            .w(px(COLUMN_WIDTH))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_lg()
            .bg(rgb(COLUMN))
            .border_1()
            .border_color(gpui::transparent_black())
            // Highlight while a card is dragged over this column...
            .drag_over::<DraggedCard>(|style, _, _, _| {
                style.bg(rgb(COLUMN_DROP)).border_color(rgb(ACCENT))
            })
            // ...and move it here when it's dropped.
            .on_drop(cx.listener(move |this, dragged: &DraggedCard, _, cx| {
                if this.board.move_to(dragged.id, status) {
                    this.changed(cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(status.label()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(count.to_string()),
                    ),
            )
            // Virtualized: GPUI asks for just the rows in view, so a column of
            // 100,000 cards costs about the same as one of 10.
            .child(
                uniform_list(
                    ("cards", ix),
                    count,
                    cx.processor(move |this, rows: Range<usize>, _window, cx| {
                        rows.filter_map(|row| this.columns[ix].get(row))
                            .filter_map(|&position| this.board.cards.get(position))
                            .map(|card| this.render_card(card, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1(),
            )
            .child(self.render_input(ix, window, cx))
    }

    /// Every card must be the same height for `uniform_list`, so titles are a
    /// single line, cut off with "…". The drag preview shows the full title.
    fn render_card(&self, card: &Card, cx: &mut Context<Self>) -> Div {
        let id = card.id;
        let dragged = DraggedCard {
            id,
            title: card.title.clone().into(),
        };

        // The bottom padding is the gap between cards.
        div().pb_2().child(
            card_shell()
                .id(("card", id))
                .debug_selector(|| format!("card-{id}"))
                .flex()
                .items_start()
                .justify_between()
                .gap_2()
                .cursor_grab()
                .hover(|style| style.bg(rgb(CARD_HOVER)))
                .on_drag(dragged, |dragged, _offset, _window, cx| {
                    cx.new(|_| dragged.clone())
                })
                .child(div().flex_1().truncate().child(card.title.clone()))
                .child(
                    div()
                        .id(("delete", id))
                        .debug_selector(|| format!("delete-{id}"))
                        .px_1()
                        .rounded_sm()
                        .text_color(rgb(MUTED))
                        .cursor_pointer()
                        .hover(|style| style.text_color(rgb(DANGER)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.board.delete(id) {
                                this.changed(cx);
                            }
                        }))
                        .child("×"),
                ),
        )
    }

    fn render_chaos_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("chaos")
            .px_3()
            .py_1()
            .rounded_md()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(BACKGROUND))
            .bg(rgb(if self.chaos { DANGER } else { ACCENT }))
            .cursor_pointer()
            .hover(|style| style.opacity(0.85))
            .on_click(cx.listener(|this, _, _, cx| {
                this.chaos = !this.chaos;
                cx.notify();
            }))
            .child(if self.chaos {
                "■ Stop chaos"
            } else {
                "▶ Chaos"
            })
    }

    fn render_input(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> Div {
        let focus = &self.input_focus[ix];
        let focused = focus.is_focused(window);
        let draft = &self.drafts[ix];
        let placeholder = if focused {
            "Type a title, then press Enter"
        } else {
            "+ Add a card"
        };

        div()
            .track_focus(focus)
            .debug_selector(|| format!("input-{ix}"))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                this.on_input_key(ix, event, window, cx)
            }))
            .flex()
            .items_center()
            .min_h(px(36.))
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if focused { ACCENT } else { BORDER }))
            .bg(rgb(BACKGROUND))
            .text_sm()
            .cursor_text()
            .overflow_hidden()
            .when(!draft.is_empty(), |el| el.child(draft.clone()))
            .when(focused, |el| {
                el.child(div().w(px(2.)).h(px(16.)).bg(rgb(ACCENT)))
            })
            .when(draft.is_empty(), |el| {
                el.child(div().text_color(rgb(MUTED)).child(placeholder))
            })
    }
}

impl Render for KanbanBoard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(perf) = &mut self.perf {
            perf.frame_started();
            // Redraw continuously, so the gap between frames is the real cost.
            window.request_animation_frame();
        }
        if self.chaos {
            self.board.chaos(&mut self.rng, CHAOS_MOVES);
            self.mark_unsaved(cx);
        }
        let build_started = Instant::now();
        self.board.index_columns(&mut self.columns);
        let card_count = self.board.cards.len();
        let perf_summary = self
            .perf
            .as_ref()
            .map(|perf| perf.summary(card_count, self.chaos));
        if let (Some(perf), Some(summary)) = (&mut self.perf, &perf_summary) {
            perf.maybe_report(summary);
        }
        let subtitle = perf_summary
            .clone()
            .unwrap_or_else(|| format!("{card_count} cards · drag to move · saves automatically"));
        let stress = self.perf.is_some();

        let element = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_6()
                    .pt_5()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_3()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Kanban"),
                            )
                            .child(div().text_sm().text_color(rgb(MUTED)).child(subtitle)),
                    )
                    .when(stress, |el| el.child(self.render_chaos_button(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .flex()
                    .gap_4()
                    .p_6()
                    .pt_4()
                    .children(
                        Status::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(ix, status)| self.render_column(ix, status, window, cx)),
                    ),
            );

        if let Some(perf) = &mut self.perf {
            perf.build = build_started.elapsed();
        }
        element
    }
}

fn main() {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(err) => {
            eprintln!("{err}\n{USAGE}");
            process::exit(2);
        }
    };

    Application::new().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "Kanban".into(),
            items: vec![MenuItem::action("Quit Kanban", Quit)],
        }]);
        // Closing the last window quits, like most single-window Mac apps.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        if let Some(after) = options.exit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(after).await;
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }

        // A stress board is generated fresh each run (so runs are comparable)
        // and saved to its own file, never over your real board.
        let (board, save_path, title) = match options.stress {
            Some(count) => (
                Board::stress(count, 42),
                board::data_file("stress.json"),
                "Kanban — stress test",
            ),
            None => {
                let path = board::default_path();
                (Board::load(&path), path, "Kanban")
            }
        };
        let stress = options.stress.is_some();
        let chaos = options.chaos;

        let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut view = KanbanBoard::new(board, save_path, window, cx);
                    if stress {
                        view.perf = Some(Perf::new());
                        view.chaos = chaos;
                    }
                    view
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &str) -> Result<Options, String> {
        Options::parse(args.split_whitespace().map(String::from))
    }

    #[test]
    fn options_parse_every_flag() {
        assert_eq!(parse(""), Ok(Options::default()));
        assert_eq!(
            parse("--stress 10000 --chaos --exit-after 6"),
            Ok(Options {
                stress: Some(10_000),
                chaos: true,
                exit_after: Some(Duration::from_secs(6)),
            })
        );
    }

    #[test]
    fn options_reject_bad_input() {
        assert!(parse("--stress").is_err());
        assert!(parse("--stress lots").is_err());
        assert!(parse("--exit-after -1").is_err());
        assert!(
            parse("--chaos").is_err(),
            "chaos must not touch the real board"
        );
        assert!(parse("--wat").is_err());
    }
}
