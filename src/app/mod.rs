mod detail;
mod keys;
mod unified;

use crate::actions::{self, ActionKind, ActionPlan};
use crate::create::{self, CreateServiceForm};
use crate::discovery;
use crate::model::{Inventory, Service, ServiceSource, ServiceStatus};
use crate::oslog::LogWindow;
use crate::ui;
use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use detail::detail_item_value;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewMode {
    Overview,
    Detail,
    Logs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailItem {
    Status,
    Source,
    Domain,
    Scope,
    Safety,
    Plist,
    BrewFormula,
    BrewStatus,
    Command,
    WorkingDirectory,
    Stdout,
    Stderr,
    RunAtLoad,
    KeepAlive,
    StartInterval,
    Schedule,
    Health,
}

impl DetailItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Source => "source",
            Self::Domain => "domain",
            Self::Scope => "scope",
            Self::Safety => "safety",
            Self::Plist => "plist",
            Self::BrewFormula => "brew formula",
            Self::BrewStatus => "brew status",
            Self::Command => "command",
            Self::WorkingDirectory => "working directory",
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::RunAtLoad => "RunAtLoad",
            Self::KeepAlive => "KeepAlive",
            Self::StartInterval => "StartInterval",
            Self::Schedule => "schedule",
            Self::Health => "health",
        }
    }
}

const DETAIL_ITEMS: [DetailItem; 17] = [
    DetailItem::Status,
    DetailItem::Source,
    DetailItem::Domain,
    DetailItem::Scope,
    DetailItem::Safety,
    DetailItem::Plist,
    DetailItem::BrewFormula,
    DetailItem::BrewStatus,
    DetailItem::Command,
    DetailItem::WorkingDirectory,
    DetailItem::Stdout,
    DetailItem::Stderr,
    DetailItem::RunAtLoad,
    DetailItem::KeepAlive,
    DetailItem::StartInterval,
    DetailItem::Schedule,
    DetailItem::Health,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogStream {
    Stdout,
    Stderr,
    Unified,
}

impl LogStream {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Unified => "os_log",
        }
    }

    fn toggle(self) -> Self {
        match self {
            Self::Stdout => Self::Stderr,
            Self::Stderr => Self::Unified,
            Self::Unified => Self::Stdout,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UnifiedLogCache {
    pub service_id: String,
    pub window: LogWindow,
    pub elapsed_ms: u128,
    pub lines: Vec<String>,
    pub error: Option<String>,
    pub predicate: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFilter {
    All,
    Launchd,
    Homebrew,
}

impl SourceFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Homebrew,
            Self::Homebrew => Self::Launchd,
            Self::Launchd => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Launchd => "launchd",
            Self::Homebrew => "brew",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusFilter {
    All,
    Running,
    Failed,
    Stopped,
}

impl StatusFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Running,
            Self::Running => Self::Failed,
            Self::Failed => Self::Stopped,
            Self::Stopped => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Stopped => "inactive",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortMode {
    Name,
    Status,
    Source,
    Health,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Status,
            Self::Status => Self::Source,
            Self::Source => Self::Health,
            Self::Health => Self::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Status => "status",
            Self::Source => "source",
            Self::Health => "health",
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub services: Vec<Service>,
    pub filtered: Vec<usize>,
    pub warnings: Vec<String>,
    pub selected: usize,
    pub viewport_start: usize,
    pub search: String,
    pub source_filter: SourceFilter,
    pub status_filter: StatusFilter,
    pub sort_mode: SortMode,
    pub show_apple: bool,
    pub warnings_only: bool,
    pub detail_selected: usize,
    pub detail_scroll: usize,
    pub log_stream: LogStream,
    pub log_scroll: usize,
    pub unified_window: LogWindow,
    pub unified_cache: Option<UnifiedLogCache>,
    pub unified_in_flight: bool,
    pub unified_fetch_requested: bool,
    pub editing_search: bool,
    pub mode: ViewMode,
    pub show_help: bool,
    pub pending_action: Option<ActionPlan>,
    pub create_form: Option<CreateServiceForm>,
    pub refresh_requested: bool,
    pub status_line: String,
    pub last_refresh: Instant,
}

impl App {
    fn new(inventory: Inventory) -> Self {
        let mut app = Self {
            services: inventory.services,
            filtered: Vec::new(),
            warnings: inventory.warnings,
            selected: 0,
            viewport_start: 0,
            search: String::new(),
            source_filter: SourceFilter::All,
            status_filter: StatusFilter::All,
            sort_mode: SortMode::Status,
            show_apple: false,
            warnings_only: false,
            detail_selected: 0,
            detail_scroll: 0,
            log_stream: LogStream::Stdout,
            log_scroll: 0,
            unified_window: LogWindow::FifteenMin,
            unified_cache: None,
            unified_in_flight: false,
            unified_fetch_requested: false,
            editing_search: false,
            mode: ViewMode::Overview,
            show_help: false,
            pending_action: None,
            create_form: None,
            refresh_requested: false,
            status_line: "inventory ready; actions require confirmation".to_string(),
            last_refresh: Instant::now(),
        };
        app.apply_filter();
        app
    }

    fn apply_inventory(&mut self, inventory: Inventory) {
        let selected_id = self.selected_service_id();
        self.services = inventory.services;
        self.warnings = inventory.warnings;
        let kept_selection = self.apply_filter_with_selection(selected_id.as_deref());
        self.last_refresh = Instant::now();
        if selected_id.is_some()
            && !kept_selection
            && matches!(self.mode, ViewMode::Detail | ViewMode::Logs)
        {
            self.mode = ViewMode::Overview;
            self.detail_scroll = 0;
            self.log_scroll = 0;
            self.status_line = format!(
                "refreshed {} services; selected service left the current view",
                self.services.len()
            );
        } else {
            self.status_line = format!("refreshed {} services", self.services.len());
        }
    }

    fn apply_filter(&mut self) {
        self.apply_filter_with_selection(None);
    }

    fn apply_filter_with_selection(&mut self, selected_id: Option<&str>) -> bool {
        let query = self.search.trim().to_lowercase();
        self.filtered = self
            .services
            .iter()
            .enumerate()
            .filter_map(|(index, service)| {
                if !matches_source(service, self.source_filter) {
                    return None;
                }
                if !matches_status(service, self.status_filter) {
                    return None;
                }
                if !self.show_apple && is_apple_service(service) {
                    return None;
                }
                if self.warnings_only && service.health.is_empty() {
                    return None;
                }
                if query.is_empty() || service.searchable_text().contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();
        sort_filtered(&mut self.filtered, &self.services, self.sort_mode);

        let restored_selection = selected_id
            .and_then(|id| {
                self.filtered
                    .iter()
                    .position(|index| self.services[*index].id == id)
            })
            .is_some_and(|position| {
                self.selected = position;
                true
            });

        if !restored_selection && self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
        if self.viewport_start >= self.filtered.len() {
            self.viewport_start = self.filtered.len().saturating_sub(1);
        }
        restored_selection
    }

    fn selected_service(&self) -> Option<&Service> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.services.get(*index))
    }

    fn selected_service_id(&self) -> Option<String> {
        self.selected_service().map(|service| service.id.clone())
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn page_down(&mut self) {
        let step = 10.min(self.filtered.len().saturating_sub(1)).max(1);
        self.selected = (self.selected + step).min(self.filtered.len().saturating_sub(1));
    }

    fn page_up(&mut self) {
        let step = 10.min(self.filtered.len().saturating_sub(1)).max(1);
        self.selected = self.selected.saturating_sub(step);
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = "search cleared".to_string();
    }

    fn open_search(&mut self) {
        self.editing_search = true;
        self.mode = ViewMode::Overview;
        self.status_line = "search mode: type query, enter to apply, esc to cancel".to_string();
    }

    fn start_quick_search(&mut self, value: char) {
        self.search.clear();
        self.search.push(value);
        self.editing_search = true;
        self.mode = ViewMode::Overview;
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = "quick search: keep typing, enter to apply, esc to cancel".to_string();
    }

    fn copy_selected_value(&mut self) {
        let Some((label, value)) = self.copy_target() else {
            self.status_line = "nothing selected to copy".to_string();
            return;
        };

        let value = value.trim();
        if value.is_empty() || value == "-" {
            self.status_line = format!("nothing to copy for {label}");
            return;
        }

        match copy_to_clipboard(value) {
            Ok(()) => self.status_line = format!("copied {label}: {value}"),
            Err(err) => self.status_line = format!("copy failed: {err}"),
        }
    }

    fn copy_target(&self) -> Option<(String, String)> {
        match self.mode {
            ViewMode::Overview => self
                .selected_service()
                .map(|service| ("service".to_string(), service.display_name.clone())),
            ViewMode::Detail => self.selected_service().map(|service| {
                let item = self.selected_detail_item();
                (item.label().to_string(), detail_item_value(service, item))
            }),
            ViewMode::Logs => self
                .selected_service()
                .map(|service| match self.log_stream {
                    LogStream::Stdout => (
                        "stdout path".to_string(),
                        service
                            .config
                            .stdout_path
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    LogStream::Stderr => (
                        "stderr path".to_string(),
                        service
                            .config
                            .stderr_path
                            .clone()
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    LogStream::Unified => (
                        "os_log predicate".to_string(),
                        self.unified_cache
                            .as_ref()
                            .map(|cache| cache.predicate.clone())
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                }),
        }
    }

    fn cycle_source_filter(&mut self) {
        self.source_filter = self.source_filter.next();
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = format!("source filter: {}", self.source_filter.label());
    }

    fn cycle_status_filter(&mut self) {
        self.status_filter = self.status_filter.next();
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = format!("status filter: {}", self.status_filter.label());
    }

    fn cycle_sort(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = format!("sort: {}", self.sort_mode.label());
    }

    fn toggle_apple(&mut self) {
        self.show_apple = !self.show_apple;
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = if self.show_apple {
            "showing Apple services".to_string()
        } else {
            "hiding Apple services".to_string()
        };
    }

    fn toggle_warnings_only(&mut self) {
        self.warnings_only = !self.warnings_only;
        self.selected = 0;
        self.viewport_start = 0;
        self.apply_filter();
        self.status_line = if self.warnings_only {
            "showing warnings only".to_string()
        } else {
            "showing all health states".to_string()
        };
    }

    fn plan_action(&mut self, kind: ActionKind) {
        let Some(service) = self.selected_service() else {
            return;
        };
        let plan = actions::plan(service, kind);
        if plan.is_blocked() {
            self.status_line = format!(
                "{} blocked for {}: {}",
                kind.label(),
                plan.service_name,
                plan.blocked_reason.as_deref().unwrap_or("blocked")
            );
            self.pending_action = Some(plan);
            return;
        }

        self.status_line = format!(
            "{} {}: press y to run `{}`",
            kind.label(),
            plan.service_name,
            plan.command_display()
        );
        self.pending_action = Some(plan);
    }

    fn cancel_action(&mut self) {
        self.pending_action = None;
        self.status_line = "action cancelled".to_string();
    }

    fn confirm_action(&mut self) {
        let Some(plan) = self.pending_action.clone() else {
            return;
        };

        if plan.is_blocked() {
            self.status_line = format!(
                "{} blocked: {}",
                plan.kind.label(),
                plan.blocked_reason.as_deref().unwrap_or("blocked")
            );
            return;
        }

        let command = plan.command_display();
        let result = actions::execute(&plan);
        self.pending_action = None;
        self.refresh_requested = true;
        if result.success {
            self.status_line = format!("ran `{command}`: {}", result.message);
        } else {
            self.status_line = format!("failed `{command}`: {}", result.message);
        }
    }

    fn open_create_form(&mut self) {
        self.create_form = Some(CreateServiceForm::new());
        self.status_line = "new user agent: edit fields, ctrl+s or F5 to save".to_string();
    }

    fn cancel_create_form(&mut self) {
        self.create_form = None;
        self.status_line = "service creation cancelled".to_string();
    }

    fn open_detail(&mut self) {
        self.mode = ViewMode::Detail;
        self.detail_selected = 0;
        self.detail_scroll = 0;
        self.status_line = "detail: up/down moves inside, enter opens selected row".to_string();
    }

    fn close_detail(&mut self) {
        self.mode = ViewMode::Overview;
        self.status_line = "detail closed".to_string();
    }

    pub fn detail_items() -> &'static [DetailItem] {
        &DETAIL_ITEMS
    }

    pub fn selected_detail_item(&self) -> DetailItem {
        DETAIL_ITEMS[self.detail_selected.min(DETAIL_ITEMS.len() - 1)]
    }

    fn move_detail_down(&mut self) {
        if self.detail_selected + 1 < DETAIL_ITEMS.len() {
            self.detail_selected += 1;
        }
    }

    fn move_detail_up(&mut self) {
        self.detail_selected = self.detail_selected.saturating_sub(1);
    }

    fn page_detail_down(&mut self) {
        self.detail_selected = (self.detail_selected + 6).min(DETAIL_ITEMS.len() - 1);
    }

    fn page_detail_up(&mut self) {
        self.detail_selected = self.detail_selected.saturating_sub(6);
    }

    fn open_selected_detail_item(&mut self) {
        match self.selected_detail_item() {
            DetailItem::Status => self.plan_action(status_detail_action(self.selected_service())),
            DetailItem::Stdout => self.open_logs(LogStream::Stdout),
            DetailItem::Stderr => self.open_logs(LogStream::Stderr),
            DetailItem::RunAtLoad => self.plan_action(ActionKind::ToggleRunAtLoad),
            DetailItem::Plist => self.plan_action(ActionKind::EditPlist),
            item => {
                self.status_line = format!("{} selected", item.label());
            }
        }
    }

    fn save_create_form(&mut self) {
        let Some(form) = &self.create_form else {
            return;
        };

        match create::write_user_agent(form) {
            Ok(outcome) => {
                self.create_form = None;
                self.refresh_requested = true;
                self.status_line = outcome.status_message();
            }
            Err(err) => {
                self.status_line = format!("create failed: {err}");
            }
        }
    }

    fn open_logs(&mut self, stream: LogStream) {
        self.mode = ViewMode::Logs;
        self.log_stream = stream;
        self.log_scroll = 0;
        self.status_line = "logs: tab cycles streams, r refresh, w window".to_string();
        if stream == LogStream::Unified {
            self.ensure_unified_fetch();
        }
    }

    fn scroll_logs_up(&mut self, amount: usize) {
        self.log_scroll = self.log_scroll.saturating_add(amount);
    }

    fn scroll_logs_down(&mut self, amount: usize) {
        self.log_scroll = self.log_scroll.saturating_sub(amount);
    }

    fn switch_log_stream(&mut self) {
        self.log_stream = self.log_stream.toggle();
        self.log_scroll = 0;
        self.status_line = format!("showing {}", self.log_stream.label());
        if self.log_stream == LogStream::Unified {
            self.ensure_unified_fetch();
        }
    }

    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let running = self
            .services
            .iter()
            .filter(|service| service.status == ServiceStatus::Running)
            .count();
        let failed = self
            .services
            .iter()
            .filter(|service| service.status == ServiceStatus::Failed)
            .count();
        let warnings = self
            .services
            .iter()
            .filter(|service| !service.health.is_empty())
            .count();
        (self.services.len(), running, failed, warnings)
    }
}

fn matches_source(service: &Service, filter: SourceFilter) -> bool {
    match filter {
        SourceFilter::All => true,
        SourceFilter::Launchd => {
            matches!(service.source, ServiceSource::Launchd | ServiceSource::Both)
        }
        SourceFilter::Homebrew => matches!(
            service.source,
            ServiceSource::Homebrew | ServiceSource::Both
        ),
    }
}

fn matches_status(service: &Service, filter: StatusFilter) -> bool {
    match filter {
        StatusFilter::All => true,
        StatusFilter::Running => service.status == ServiceStatus::Running,
        StatusFilter::Failed => service.status == ServiceStatus::Failed,
        StatusFilter::Stopped => matches!(
            service.status,
            ServiceStatus::Scheduled
                | ServiceStatus::Stopped
                | ServiceStatus::Unloaded
                | ServiceStatus::Disabled
        ),
    }
}

fn status_detail_action(service: Option<&Service>) -> ActionKind {
    match service.map(|service| &service.status) {
        Some(ServiceStatus::Running | ServiceStatus::Scheduled) => ActionKind::Stop,
        Some(ServiceStatus::Stopped | ServiceStatus::Unloaded) => ActionKind::Start,
        Some(ServiceStatus::Failed) => ActionKind::Restart,
        Some(ServiceStatus::Disabled) => ActionKind::ToggleEnabled,
        Some(ServiceStatus::Unknown) | None => ActionKind::Restart,
    }
}

fn is_apple_service(service: &Service) -> bool {
    service.label.starts_with("com.apple.")
        || service
            .plist_path
            .as_ref()
            .is_some_and(|path| path.starts_with("/System/Library"))
}

fn sort_filtered(filtered: &mut [usize], services: &[Service], sort_mode: SortMode) {
    filtered.sort_by(|left, right| {
        let left = &services[*left];
        let right = &services[*right];
        match sort_mode {
            SortMode::Name => left
                .display_name
                .to_lowercase()
                .cmp(&right.display_name.to_lowercase()),
            SortMode::Status => status_rank(&left.status)
                .cmp(&status_rank(&right.status))
                .then_with(|| left.display_name.cmp(&right.display_name)),
            SortMode::Source => left
                .source
                .to_string()
                .cmp(&right.source.to_string())
                .then_with(|| left.display_name.cmp(&right.display_name)),
            SortMode::Health => right
                .health
                .len()
                .cmp(&left.health.len())
                .then_with(|| left.display_name.cmp(&right.display_name)),
        }
    });
}

fn status_rank(status: &ServiceStatus) -> u8 {
    match status {
        ServiceStatus::Failed => 0,
        ServiceStatus::Disabled => 1,
        ServiceStatus::Running => 2,
        ServiceStatus::Scheduled => 3,
        ServiceStatus::Stopped => 4,
        ServiceStatus::Unloaded => 5,
        ServiceStatus::Unknown => 6,
    }
}

pub fn run() -> Result<()> {
    let inventory = discovery::load_inventory();
    let mut app = App::new(inventory);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    let (refresh_tx, refresh_rx) = mpsc::channel();
    let (unified_tx, unified_rx) = mpsc::channel();
    let mut refresh_in_progress = false;

    loop {
        if let Some(inventory) = receive_refresh(&refresh_rx) {
            refresh_in_progress = false;
            app.apply_inventory(inventory);
        }

        if let Some(result) = unified::drain_unified(&unified_rx) {
            app.apply_unified_result(result);
        }

        if app.unified_fetch_requested && !app.unified_in_flight {
            app.unified_fetch_requested = false;
            unified::spawn_unified_fetch(app, &unified_tx);
        }

        terminal.draw(|frame| ui::draw(frame, app))?;

        if should_refresh(app, refresh_in_progress) {
            app.refresh_requested = false;
            app.last_refresh = Instant::now();
            app.status_line = "refreshing in background...".to_string();
            refresh_in_progress = true;
            spawn_refresh(refresh_tx.clone());
        }

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        if keys::handle_key(app, key) {
            break;
        }
    }

    Ok(())
}

fn receive_refresh(refresh_rx: &Receiver<Inventory>) -> Option<Inventory> {
    let mut latest = None;
    while let Ok(inventory) = refresh_rx.try_recv() {
        latest = Some(inventory);
    }
    latest
}

fn should_refresh(app: &App, refresh_in_progress: bool) -> bool {
    if refresh_in_progress
        || app.editing_search
        || app.show_help
        || app.pending_action.is_some()
        || app.create_form.is_some()
    {
        return false;
    }
    app.refresh_requested || app.last_refresh.elapsed() >= REFRESH_INTERVAL
}

fn spawn_refresh(refresh_tx: Sender<Inventory>) {
    thread::spawn(move || {
        let inventory = discovery::load_inventory();
        let _ = refresh_tx.send(inventory);
    });
}

fn copy_to_clipboard(value: &str) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(anyhow::Error::from)?;

    let mut stdin = child.stdin.take().context("pbcopy stdin unavailable")?;
    stdin.write_all(value.as_bytes())?;
    drop(stdin);

    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("pbcopy exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LaunchConfig, SafetyLevel, ServiceScope};
    use std::path::PathBuf;

    fn service(label: &str, status: ServiceStatus) -> Service {
        Service {
            id: format!("gui/501:{label}"),
            label: label.to_string(),
            display_name: label.to_string(),
            source: ServiceSource::Launchd,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: Some(PathBuf::from(format!("/tmp/{label}.plist"))),
            config: LaunchConfig::empty(),
            pid: if status == ServiceStatus::Running {
                Some(123)
            } else {
                None
            },
            exit_code: None,
            status,
            enabled: Some(true),
            loaded: Some(true),
            brew_formula: None,
            brew_status: None,
            safety_level: SafetyLevel::UserWritable,
            health: Vec::new(),
        }
    }

    fn inventory(services: Vec<Service>) -> Inventory {
        Inventory {
            services,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn refresh_preserves_selected_service_identity_after_resort() {
        let mut app = App::new(inventory(vec![
            service("com.example.selected", ServiceStatus::Running),
            service("com.example.other", ServiceStatus::Stopped),
        ]));
        app.open_detail();

        app.apply_inventory(inventory(vec![
            service("com.example.selected", ServiceStatus::Unloaded),
            service("com.example.other", ServiceStatus::Running),
        ]));

        assert_eq!(app.mode, ViewMode::Detail);
        assert_eq!(
            app.selected_service_id().as_deref(),
            Some("gui/501:com.example.selected")
        );
    }

    #[test]
    fn refresh_closes_detail_when_selected_service_leaves_filter() {
        let mut app = App::new(inventory(vec![
            service("com.example.selected", ServiceStatus::Running),
            service("com.example.other", ServiceStatus::Running),
        ]));
        app.status_filter = StatusFilter::Running;
        app.apply_filter();
        let Some(position) = app
            .filtered
            .iter()
            .position(|index| app.services[*index].id == "gui/501:com.example.selected")
        else {
            panic!("selected service missing from test inventory");
        };
        app.selected = position;
        app.open_detail();

        app.apply_inventory(inventory(vec![
            service("com.example.selected", ServiceStatus::Unloaded),
            service("com.example.other", ServiceStatus::Running),
        ]));

        assert_eq!(app.mode, ViewMode::Overview);
        assert_eq!(
            app.selected_service_id().as_deref(),
            Some("gui/501:com.example.other")
        );
    }
}
