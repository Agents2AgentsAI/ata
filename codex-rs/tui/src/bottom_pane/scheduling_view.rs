use codex_protocol::protocol::SchedulingCronRow;
use codex_protocol::protocol::SchedulingLoopRow;
use codex_protocol::protocol::SchedulingMonitorRow;
use codex_protocol::protocol::SchedulingTaskKind;
use codex_protocol::protocol::SchedulingTasksSnapshotEvent;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Widget;
use std::cell::Cell;
use std::time::Duration;
use std::time::Instant;

use crate::app_command::AppCommand;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::key_hint;
use crate::render::Insets;
use crate::render::RectExt as _;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::style::user_message_style;
use crate::tui::FrameRequester;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// Phase 3, Slice 1b: scheduling-inspection panel backed by a real snapshot.
///
/// The panel starts in a "loading" state, then re-renders once the host pushes
/// a [`SchedulingTasksSnapshotEvent`] in via [`Self::set_snapshot`]. The
/// snapshot arrives through `BottomPane::notify_scheduling_snapshot` after the
/// TUI dispatches `AppCommand::ListSchedulingTasks` and the session replies
/// over the app-server `scheduling/tasks/snapshot` notification.
pub(crate) struct SchedulingView {
    complete: bool,
    snapshot: Option<SchedulingTasksSnapshotEvent>,
    footer_hint: Line<'static>,
    /// Set when the view is mounted by the chatwidget; absent in unit tests
    /// that bypass the host (no auto-refresh in that case).
    auto_refresh: Option<AutoRefresh>,
    /// Index into the flattened row list (cron → monitor → loop). Used by the
    /// `j`/`k`/arrow keys and the `d` shortcut.
    selected_index: usize,
}

struct AutoRefresh {
    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,
    /// When we last sent an `AppCommand::ListSchedulingTasks` op. `Cell`
    /// because `render` is `&self`.
    last_dispatch: Cell<Option<Instant>>,
}

impl SchedulingView {
    pub(crate) fn new() -> Self {
        Self {
            complete: false,
            snapshot: None,
            footer_hint: scheduling_popup_hint_line(),
            auto_refresh: None,
            selected_index: 0,
        }
    }

    /// Flatten the snapshot into a `(kind, task_id)` list in display order
    /// (cron → monitor → loop). Returns an empty vec when the snapshot is
    /// missing, scheduling is disabled, or there are no rows.
    fn flat_rows(&self) -> Vec<(SchedulingTaskKind, String)> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        if !snapshot.scheduling_enabled {
            return Vec::new();
        }
        let mut rows = Vec::new();
        for row in &snapshot.cron_jobs {
            rows.push((SchedulingTaskKind::Cron, row.task_id.clone()));
        }
        for row in &snapshot.monitors {
            rows.push((SchedulingTaskKind::Monitor, row.task_id.clone()));
        }
        for row in &snapshot.loops {
            rows.push((SchedulingTaskKind::Loop, row.task_id.clone()));
        }
        rows
    }

    fn clamped_selected_index(&self) -> usize {
        let len = self.flat_rows().len();
        if len == 0 {
            0
        } else {
            self.selected_index.min(len - 1)
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.flat_rows().len();
        if len == 0 {
            self.selected_index = 0;
            return;
        }
        let current = self.clamped_selected_index() as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.selected_index = next;
    }

    fn delete_selected(&mut self) {
        let rows = self.flat_rows();
        if rows.is_empty() {
            return;
        }
        let idx = self.clamped_selected_index();
        let (kind, task_id) = rows[idx].clone();
        let Some(refresh) = &self.auto_refresh else {
            return; // no host wired up (tests)
        };
        refresh
            .app_event_tx
            .send(AppEvent::CodexOp(AppCommand::DeleteSchedulingTask {
                task_id,
                kind,
            }));
        // The server emits a fresh snapshot inside the delete handler, but
        // also reset the auto-refresh stamp so the next render dispatches an
        // additional list — covers the race where the server snapshot races
        // the next tick.
        refresh.last_dispatch.set(None);
    }

    /// Enable 1-second auto-refresh while the panel is visible. The host
    /// chatwidget should call this right after constructing the view so the
    /// snapshot stays live; tests omit it.
    pub(crate) fn with_auto_refresh(
        mut self,
        app_event_tx: AppEventSender,
        frame_requester: FrameRequester,
    ) -> Self {
        self.auto_refresh = Some(AutoRefresh {
            app_event_tx,
            frame_requester,
            last_dispatch: Cell::new(None),
        });
        self
    }

    fn set_snapshot(&mut self, snapshot: SchedulingTasksSnapshotEvent) {
        self.snapshot = Some(snapshot);
    }

    /// Send a new snapshot request if we have an auto-refresh handle and the
    /// last dispatch was at least `REFRESH_INTERVAL` ago. Always schedules
    /// the next render so we keep ticking while visible.
    fn tick_auto_refresh(&self) {
        let Some(refresh) = &self.auto_refresh else {
            return;
        };
        let now = Instant::now();
        let should_dispatch = match refresh.last_dispatch.get() {
            Some(prev) => now.duration_since(prev) >= REFRESH_INTERVAL,
            None => true,
        };
        if should_dispatch {
            refresh.last_dispatch.set(Some(now));
            refresh
                .app_event_tx
                .send(AppEvent::CodexOp(AppCommand::ListSchedulingTasks));
        }
        refresh.frame_requester.schedule_frame_in(REFRESH_INTERVAL);
    }

    fn header(&self) -> Box<dyn Renderable> {
        let mut header = ColumnRenderable::new();
        header.push(Line::from("Scheduling tasks in this session".bold()));
        header.push(Line::from(
            "Active cron jobs, monitors, and loops for this thread.".dim(),
        ));
        // Heartbeat row so the user can see auto-refresh is alive even when
        // no row data has changed yet.
        if let Some(refresh) = &self.auto_refresh {
            let stamp = match refresh.last_dispatch.get() {
                Some(_) => chrono::Local::now().format("%H:%M:%S").to_string(),
                None => "—".to_string(),
            };
            header.push(Line::from(format!("Updated: {stamp}").dim()));
        }
        Box::new(header)
    }

    fn body(&self) -> Box<dyn Renderable> {
        let mut body = ColumnRenderable::new();
        let Some(snapshot) = &self.snapshot else {
            body.push(Line::from(""));
            body.push(Line::from("Loading…".dim()));
            return Box::new(body);
        };

        if !snapshot.scheduling_enabled {
            body.push(Line::from(""));
            body.push(Line::from(
                "Scheduling is not enabled. Toggle it on in /experimental.".dim(),
            ));
            return Box::new(body);
        }

        // Index that increments across cron → monitor → loop so the
        // selection marker tracks the same flat order as `flat_rows()`.
        let selected = self.clamped_selected_index();
        let mut idx: usize = 0;

        body.push(Line::from(""));
        body.push(Line::from(format!("Cron ({})", snapshot.cron_jobs.len()).bold()));
        if snapshot.cron_jobs.is_empty() {
            body.push(Line::from("  (none)".dim()));
        } else {
            for row in &snapshot.cron_jobs {
                for line in cron_row_lines(row, idx == selected) {
                    body.push(line);
                }
                idx += 1;
            }
        }

        body.push(Line::from(""));
        body.push(Line::from(format!("Monitors ({})", snapshot.monitors.len()).bold()));
        if snapshot.monitors.is_empty() {
            body.push(Line::from("  (none)".dim()));
        } else {
            for row in &snapshot.monitors {
                for line in monitor_row_lines(row, idx == selected) {
                    body.push(line);
                }
                idx += 1;
            }
        }

        body.push(Line::from(""));
        body.push(Line::from(format!("Loops ({})", snapshot.loops.len()).bold()));
        if snapshot.loops.is_empty() {
            body.push(Line::from("  (none)".dim()));
        } else {
            for row in &snapshot.loops {
                for line in loop_row_lines(row, idx == selected) {
                    body.push(line);
                }
                idx += 1;
            }
        }

        Box::new(body)
    }
}

impl BottomPaneView for SchedulingView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.complete = true;
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_selection(1);
            }
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.move_selection(-1);
            }
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.delete_selected();
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn handle_scheduling_snapshot(&mut self, snapshot: SchedulingTasksSnapshotEvent) {
        self.set_snapshot(snapshot);
    }
}

impl Renderable for SchedulingView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        // Drive periodic snapshot fetches while the view is on screen.
        self.tick_auto_refresh();

        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        Block::default()
            .style(user_message_style())
            .render(content_area, buf);

        let inner = content_area.inset(Insets::vh(/*v*/ 1, /*h*/ 2));
        let header = self.header();
        let body = self.body();
        let header_height = header.desired_height(inner.width);
        let body_height = body.desired_height(inner.width);
        let [header_area, _, body_area] = Layout::vertical([
            Constraint::Max(header_height),
            Constraint::Max(1),
            Constraint::Max(body_height),
        ])
        .areas(inner);

        header.render(header_area, buf);
        body.render(body_area, buf);

        let hint_area = Rect {
            x: footer_area.x + 2,
            y: footer_area.y,
            width: footer_area.width.saturating_sub(2),
            height: footer_area.height,
        };
        self.footer_hint.clone().dim().render(hint_area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let inner_width = width.saturating_sub(4);
        let header_height = self.header().desired_height(inner_width);
        let body_height = self.body().desired_height(inner_width);
        header_height
            .saturating_add(body_height)
            .saturating_add(4)
    }
}

fn scheduling_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        key_hint::plain(KeyCode::Up).into(),
        "/".into(),
        key_hint::plain(KeyCode::Down).into(),
        " select · ".into(),
        key_hint::plain(KeyCode::Char('d')).into(),
        " delete · ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " close".into(),
    ])
}

/// Renders one task as two lines: a compact header (`id status prompt`) and
/// an indented details line (`counters · timing`). Keeps both inside typical
/// terminal widths so nothing is hidden on the right.
fn row_marker(selected: bool) -> &'static str {
    if selected { "▸ " } else { "  " }
}

fn row_head(marker: &str, short_id: &str, status: &str, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{marker}{short_id}  [")),
        status_span(status),
        Span::raw(format!("]  {label}")),
    ])
}

/// Map a status string to a colored, padded span. Colors:
/// - Pending → cyan (waiting, neutral)
/// - Running → yellow (in flight)
/// - Completed → green (success, dim so it recedes)
/// - Failed → red
/// - Killed → dim gray (user stopped it)
/// - Interrupted → magenta (subprocess died with prior session)
fn status_span(status: &str) -> Span<'static> {
    let padded = pad_status(status);
    match status {
        "Pending" => padded.cyan(),
        "Running" => padded.yellow(),
        "Completed" => padded.green().dim(),
        "Failed" => padded.red(),
        "Killed" => padded.dim(),
        "Interrupted" => padded.magenta(),
        _ => Span::raw(padded),
    }
}

fn cron_row_lines(row: &SchedulingCronRow, selected: bool) -> [Line<'static>; 2] {
    let short_id = short_task_id(&row.task_id);
    let prompt = truncate(&row.prompt, 50);
    let head = row_head(row_marker(selected), &short_id, &row.status, &prompt);
    let next = row
        .next_fire_at
        .as_deref()
        .and_then(relative_time)
        .unwrap_or_else(|| "—".to_string());
    let details = Line::from(
        format!("      fired {} · next {}", row.fire_count, next).dim(),
    );
    [head, details]
}

fn monitor_row_lines(row: &SchedulingMonitorRow, selected: bool) -> [Line<'static>; 2] {
    let short_id = short_task_id(&row.task_id);
    let cmd = truncate(&row.command, 60);
    let head = row_head(row_marker(selected), &short_id, &row.status, &cmd);
    let details = Line::from(
        format!("      lines {}", row.lines_emitted).dim(),
    );
    [head, details]
}

fn loop_row_lines(row: &SchedulingLoopRow, selected: bool) -> [Line<'static>; 2] {
    let short_id = short_task_id(&row.task_id);
    let prompt = truncate(&row.prompt, 50);
    let head = row_head(row_marker(selected), &short_id, &row.status, &prompt);
    let interval = match row.interval_seconds {
        Some(s) => format!("{s}s"),
        None => "dynamic".to_string(),
    };
    let next = row
        .next_wakeup_at
        .as_deref()
        .and_then(relative_time)
        .unwrap_or_else(|| "—".to_string());
    let details = Line::from(
        format!(
            "      iter {} · every {interval} · next {next}",
            row.iteration_count
        )
        .dim(),
    );
    [head, details]
}

fn short_task_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn pad_status(status: &str) -> String {
    // Pad to a fixed width so heads line up.
    let width = 9;
    if status.chars().count() >= width {
        status.chars().take(width).collect()
    } else {
        let mut out = status.to_string();
        for _ in status.chars().count()..width {
            out.push(' ');
        }
        out
    }
}

/// Convert an RFC 3339 timestamp into a short relative string like `in 25s`
/// / `in 2m` / `1m ago`. Returns `None` if the string can't be parsed.
fn relative_time(rfc3339: &str) -> Option<String> {
    let target = chrono::DateTime::parse_from_rfc3339(rfc3339).ok()?;
    let now = chrono::Utc::now();
    let delta = target.signed_duration_since(now);
    let secs = delta.num_seconds();
    let abs = secs.unsigned_abs();
    let label = if abs < 60 {
        format!("{abs}s")
    } else if abs < 3600 {
        format!("{}m", abs / 60)
    } else {
        format!("{}h", abs / 3600)
    };
    Some(if secs >= 0 {
        format!("in {label}")
    } else {
        format!("{label} ago")
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_string(view: &SchedulingView, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| view.render(f.area(), f.buffer_mut()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn loading_state_renders_placeholder() {
        let view = SchedulingView::new();
        let out = render_to_string(&view, 80, 10);
        assert!(out.contains("Scheduling tasks"), "header missing: {out}");
        assert!(out.contains("Loading"), "loading placeholder missing: {out}");
    }

    #[test]
    fn snapshot_renders_cron_monitor_and_loop_rows() {
        let mut view = SchedulingView::new();
        view.set_snapshot(SchedulingTasksSnapshotEvent {
            cron_jobs: vec![SchedulingCronRow {
                task_id: "c1".into(),
                cron_expr: "* * * * *".into(),
                prompt: "say hi".into(),
                status: "Scheduled".into(),
                fire_count: 3,
                last_fired_at: None,
                next_fire_at: Some("2026-05-12T12:00:00Z".into()),
            }],
            monitors: vec![SchedulingMonitorRow {
                task_id: "m1".into(),
                command: "tail -f log".into(),
                status: "Running".into(),
                lines_emitted: 42,
                started_at: None,
                stopped_at: None,
            }],
            loops: vec![SchedulingLoopRow {
                task_id: "l1".into(),
                prompt: "poll status".into(),
                interval_seconds: Some(5),
                status: "Running".into(),
                iteration_count: 7,
                last_iter_at: None,
                next_wakeup_at: None,
            }],
            scheduling_enabled: true,
        });
        let out = render_to_string(&view, 100, 20);
        assert!(out.contains("Cron (1)"), "cron header missing: {out}");
        assert!(out.contains("c1"), "cron row missing: {out}");
        assert!(out.contains("Monitors (1)"), "monitors header missing: {out}");
        assert!(out.contains("m1"), "monitor row missing: {out}");
        assert!(out.contains("Loops (1)"), "loops header missing: {out}");
        assert!(out.contains("l1"), "loop row missing: {out}");
        assert!(out.contains("every 5s"), "interval missing: {out}");
    }

    #[test]
    fn snapshot_disabled_shows_off_message() {
        let mut view = SchedulingView::new();
        view.set_snapshot(SchedulingTasksSnapshotEvent {
            cron_jobs: vec![],
            monitors: vec![],
            loops: vec![],
            scheduling_enabled: false,
        });
        let out = render_to_string(&view, 80, 12);
        assert!(out.contains("Scheduling is not enabled"), "{out}");
    }
}
