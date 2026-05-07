use super::{App, UnifiedLogCache};
use crate::oslog::{self, UnifiedLogResult};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

pub type UnifiedFetchTx = Sender<UnifiedLogResult>;

pub fn drain_unified(rx: &Receiver<UnifiedLogResult>) -> Option<UnifiedLogResult> {
    let mut latest = None;
    while let Ok(result) = rx.try_recv() {
        latest = Some(result);
    }
    latest
}

pub fn spawn_unified_fetch(app: &mut App, tx: &UnifiedFetchTx) {
    let window = app.unified_window;
    let prepared = app.selected_service().map(|service| {
        let id = service.id.clone();
        let display = service.display_name.clone();
        let query = oslog::build_query(service, window);
        (id, display, query)
    });

    let Some((service_id, display_name, query)) = prepared else {
        app.unified_cache = Some(UnifiedLogCache {
            service_id: String::new(),
            window,
            elapsed_ms: 0,
            lines: Vec::new(),
            error: Some("no service selected".to_string()),
            predicate: String::new(),
        });
        return;
    };

    let Some(query) = query else {
        app.unified_cache = Some(UnifiedLogCache {
            service_id,
            window,
            elapsed_ms: 0,
            lines: Vec::new(),
            error: Some("label has characters unsafe for log predicate".to_string()),
            predicate: String::new(),
        });
        return;
    };

    app.unified_in_flight = true;
    app.status_line = format!("fetching os_log for {display_name} ({})", window.label());

    let tx = tx.clone();
    thread::spawn(move || {
        let result = oslog::fetch(service_id, query);
        let _ = tx.send(result);
    });
}

impl App {
    pub(super) fn apply_unified_result(&mut self, result: UnifiedLogResult) {
        self.unified_in_flight = false;
        let predicate = oslog::build_predicate(&result.query);
        let lines = result.lines.len();
        let elapsed = result.elapsed_ms;
        let window_label = result.query.window.label();
        let error_summary = result.error.clone();
        self.unified_cache = Some(UnifiedLogCache {
            service_id: result.service_id,
            window: result.query.window,
            elapsed_ms: result.elapsed_ms,
            lines: result.lines,
            error: result.error,
            predicate,
        });
        self.log_scroll = 0;
        self.status_line = match error_summary {
            Some(err) => format!("os_log error ({window_label}): {err}"),
            None => format!("os_log: {lines} lines from last {window_label} in {elapsed}ms"),
        };
    }

    pub fn refresh_unified_log(&mut self) {
        self.unified_cache = None;
        self.unified_fetch_requested = true;
        self.status_line = "refreshing os_log...".to_string();
    }

    pub fn cycle_unified_window(&mut self) {
        self.unified_window = self.unified_window.next();
        self.unified_cache = None;
        self.unified_fetch_requested = true;
        self.status_line = format!("os_log window: {}", self.unified_window.label());
    }

    pub(super) fn ensure_unified_fetch(&mut self) {
        let Some(service_id) = self.selected_service().map(|s| s.id.clone()) else {
            return;
        };
        let fresh = self
            .unified_cache
            .as_ref()
            .is_some_and(|c| c.service_id == service_id && c.window == self.unified_window);
        if !fresh && !self.unified_in_flight {
            self.unified_fetch_requested = true;
        }
    }
}
