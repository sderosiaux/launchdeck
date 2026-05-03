use crate::actions::{self, ActionKind, ActionPlan};
use crate::discovery;
use crate::model::{Inventory, Service, ServiceSource, ServiceStatus};
use crate::ui;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;
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
            Self::Stopped => "stopped",
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
    pub editing_search: bool,
    pub mode: ViewMode,
    pub pending_action: Option<ActionPlan>,
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
            sort_mode: SortMode::Name,
            show_apple: false,
            warnings_only: false,
            editing_search: false,
            mode: ViewMode::Overview,
            pending_action: None,
            refresh_requested: false,
            status_line: "inventory ready; actions require confirmation".to_string(),
            last_refresh: Instant::now(),
        };
        app.apply_filter();
        app
    }

    fn apply_inventory(&mut self, inventory: Inventory) {
        self.services = inventory.services;
        self.warnings = inventory.warnings;
        self.apply_filter();
        self.last_refresh = Instant::now();
        self.status_line = format!("refreshed {} services", self.services.len());
    }

    fn apply_filter(&mut self) {
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
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
        if self.viewport_start >= self.filtered.len() {
            self.viewport_start = self.filtered.len().saturating_sub(1);
        }
    }

    fn selected_service(&self) -> Option<&Service> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.services.get(*index))
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
            ServiceStatus::Stopped | ServiceStatus::Unloaded | ServiceStatus::Disabled
        ),
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
        ServiceStatus::Stopped => 3,
        ServiceStatus::Unloaded => 4,
        ServiceStatus::Unknown => 5,
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
    let mut refresh_in_progress = false;

    loop {
        if let Some(inventory) = receive_refresh(&refresh_rx) {
            refresh_in_progress = false;
            app.apply_inventory(inventory);
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

        if handle_key(app, key) {
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
    if refresh_in_progress || app.editing_search || app.pending_action.is_some() {
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

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    if app.pending_action.is_some() {
        match key.code {
            KeyCode::Char('y') => app.confirm_action(),
            KeyCode::Char('n') | KeyCode::Esc => app.cancel_action(),
            _ => {}
        }
        return false;
    }

    if app.editing_search {
        return handle_search_key(app, key);
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('/') => {
            app.editing_search = true;
            app.mode = ViewMode::Overview;
            app.status_line = "search mode: type query, enter to apply, esc to cancel".to_string();
        }
        KeyCode::Char('c') => app.clear_search(),
        KeyCode::Char('f') => app.cycle_source_filter(),
        KeyCode::Char('F') => app.cycle_status_filter(),
        KeyCode::Char('o') => app.cycle_sort(),
        KeyCode::Char('a') => app.toggle_apple(),
        KeyCode::Char('w') => app.toggle_warnings_only(),
        KeyCode::Char('r') => {
            app.refresh_requested = true;
            app.status_line = "refresh requested".to_string();
        }
        KeyCode::Char('j') | KeyCode::Down => app.move_down(),
        KeyCode::Char('k') | KeyCode::Up => app.move_up(),
        KeyCode::PageDown => app.page_down(),
        KeyCode::PageUp => app.page_up(),
        KeyCode::Enter => {
            if app.selected_service().is_some() {
                app.mode = ViewMode::Detail;
            }
        }
        KeyCode::Char('l') => {
            if app.selected_service().is_some() {
                app.mode = ViewMode::Logs;
            }
        }
        KeyCode::Esc => app.mode = ViewMode::Overview,
        KeyCode::Char('s') => app.plan_action(ActionKind::Start),
        KeyCode::Char('x') => app.plan_action(ActionKind::Stop),
        KeyCode::Char('R') => app.plan_action(ActionKind::Restart),
        KeyCode::Char('e') => app.plan_action(ActionKind::ToggleEnabled),
        KeyCode::Char('n') => {
            app.status_line = "service creation is not implemented yet".to_string();
        }
        _ => {}
    }

    false
}

fn handle_search_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.editing_search = false;
            app.status_line = "search cancelled".to_string();
        }
        KeyCode::Enter => {
            app.editing_search = false;
            app.apply_filter();
            app.status_line = format!("{} services match", app.filtered.len());
        }
        KeyCode::Backspace => {
            app.search.pop();
            app.selected = 0;
            app.viewport_start = 0;
            app.apply_filter();
        }
        KeyCode::Char(value) => {
            app.search.push(value);
            app.selected = 0;
            app.viewport_start = 0;
            app.apply_filter();
        }
        _ => {}
    }
    false
}
