use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceSource {
    Launchd,
    Homebrew,
    Both,
}

impl fmt::Display for ServiceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launchd => write!(f, "launchd"),
            Self::Homebrew => write!(f, "brew"),
            Self::Both => write!(f, "both"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceScope {
    UserAgent,
    GlobalAgent,
    SystemDaemon,
}

impl ServiceScope {
    pub fn domain(&self, uid: u32) -> String {
        match self {
            Self::UserAgent | Self::GlobalAgent => format!("gui/{uid}"),
            Self::SystemDaemon => "system".to_string(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::UserAgent => "user agent",
            Self::GlobalAgent => "global agent",
            Self::SystemDaemon => "system",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceStatus {
    Running,
    Scheduled,
    Stopped,
    Failed,
    Unloaded,
    Disabled,
    Unknown,
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Scheduled => write!(f, "scheduled"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
            Self::Unloaded => write!(f, "unloaded"),
            Self::Disabled => write!(f, "disabled"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SafetyLevel {
    UserWritable,
    AdminRequired,
    ReadonlySystem,
    ProtectedVendor,
}

impl fmt::Display for SafetyLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserWritable => write!(f, "user"),
            Self::AdminRequired => write!(f, "admin"),
            Self::ReadonlySystem => write!(f, "readonly"),
            Self::ProtectedVendor => write!(f, "vendor"),
        }
    }
}

/// Which tool put this job on the machine. Answers "who do I go edit to change
/// this for good", which `launchctl` cannot tell you: reinstalling a Homebrew
/// formula or re-running `nix-darwin switch` will silently undo a plist edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    Homebrew,
    UserPlist,
    Mise,
    Nix,
    VendorApp,
    System,
    RuntimeOnly,
    Unknown,
}

impl Provenance {
    pub fn label(self) -> &'static str {
        match self {
            Self::Homebrew => "homebrew",
            Self::UserPlist => "user-plist",
            Self::Mise => "mise",
            Self::Nix => "nix",
            Self::VendorApp => "vendor-app",
            Self::System => "system",
            Self::RuntimeOnly => "runtime-only",
            Self::Unknown => "unknown",
        }
    }

    /// How to change this service permanently, rather than just until the owning
    /// tool next runs.
    pub fn change_hint(self) -> &'static str {
        match self {
            Self::Homebrew => "manage with `brew services`",
            Self::UserPlist => "edit the plist or use `launchctl`",
            Self::Mise => "edit mise.toml, then re-run the mise bootstrap",
            Self::Nix => "edit the Nix config, then rebuild/switch",
            Self::VendorApp => "change it from the owning app's settings",
            Self::System => "Apple-managed; inspect only",
            Self::RuntimeOnly => "no plist on disk; inspect before changing",
            Self::Unknown => "origin unknown; inspect before changing",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    High,
    Medium,
    Guess,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Guess => "guess",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Origin {
    pub kind: Provenance,
    pub confidence: Confidence,
    /// Why we picked this label, so the classification stays auditable.
    pub evidence: Vec<String>,
}

impl Origin {
    pub fn unknown() -> Self {
        Self {
            kind: Provenance::Unknown,
            confidence: Confidence::Guess,
            evidence: Vec::new(),
        }
    }

    pub fn summary(&self) -> String {
        format!("{} ({})", self.kind.label(), self.confidence.label())
    }
}

/// What actually needs root, per axis. `launchd` scope and file ownership are
/// independent: an agent in `/Library/LaunchAgents` is root-owned on disk but runs
/// in the caller's `gui/<uid>` domain, so starting it needs no privileges at all
/// while deleting its plist does.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ElevationNeeds {
    /// Driving `launchctl` in this service's domain needs root.
    pub runtime: bool,
    /// Editing the plist in place needs root.
    pub plist_write: bool,
    /// Removing the plist from its directory needs root.
    pub plist_remove: bool,
}

impl ElevationNeeds {
    pub fn none() -> Self {
        Self::default()
    }

    /// Human summary for the detail view: which axes actually need root.
    pub fn summary(&self) -> String {
        let mut needs = Vec::new();
        if self.runtime {
            needs.push("launchctl");
        }
        if self.plist_write {
            needs.push("plist edit");
        }
        if self.plist_remove {
            needs.push("plist delete");
        }
        if needs.is_empty() {
            "none needed".to_string()
        } else {
            format!("sudo for {}", needs.join(", "))
        }
    }
}

#[derive(Clone, Debug)]
pub struct CalendarSchedule {
    pub minute: Option<u64>,
    pub hour: Option<u64>,
    pub day: Option<u64>,
    pub weekday: Option<u64>,
    pub month: Option<u64>,
}

impl CalendarSchedule {
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();

        if self.month.is_none() && self.day.is_none() && self.weekday.is_none() {
            match (self.hour, self.minute) {
                (Some(hour), Some(minute)) => return format!("{hour:02}:{minute:02}"),
                (Some(hour), None) => return format!("{hour:02}:*"),
                (None, Some(minute)) => return format!("*:{minute:02}"),
                (None, None) => return "calendar".to_string(),
            }
        }

        if let Some(month) = self.month {
            parts.push(format!("mo{month}"));
        }
        if let Some(day) = self.day {
            parts.push(format!("d{day}"));
        }
        if let Some(weekday) = self.weekday {
            parts.push(weekday_name(weekday).to_string());
        }
        match (self.hour, self.minute) {
            (Some(hour), Some(minute)) => parts.push(format!("{hour:02}:{minute:02}")),
            (Some(hour), None) => parts.push(format!("{hour:02}:*")),
            (None, Some(minute)) => parts.push(format!("*:{minute:02}")),
            (None, None) => {}
        }

        if parts.is_empty() {
            "calendar".to_string()
        } else {
            parts.join(" ")
        }
    }
}

#[derive(Clone, Debug)]
pub struct LaunchConfig {
    pub program: Option<String>,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub run_at_load: Option<bool>,
    pub keep_alive: Option<String>,
    pub start_interval: Option<u64>,
    pub start_calendar_intervals: Vec<CalendarSchedule>,
}

impl LaunchConfig {
    pub fn empty() -> Self {
        Self {
            program: None,
            arguments: Vec::new(),
            working_directory: None,
            stdout_path: None,
            stderr_path: None,
            run_at_load: None,
            keep_alive: None,
            start_interval: None,
            start_calendar_intervals: Vec::new(),
        }
    }

    pub fn command_preview(&self) -> String {
        if !self.arguments.is_empty() {
            self.arguments.join(" ")
        } else {
            self.program.clone().unwrap_or_else(|| "-".to_string())
        }
    }

    pub fn schedule_summary(&self) -> String {
        let mut schedules = Vec::new();
        if let Some(interval) = self.start_interval {
            schedules.push(format_interval(interval));
        }
        schedules.extend(
            self.start_calendar_intervals
                .iter()
                .map(CalendarSchedule::describe),
        );

        if schedules.is_empty() {
            "-".to_string()
        } else {
            schedules.join(", ")
        }
    }
}

fn format_interval(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    if seconds >= DAY && seconds.is_multiple_of(DAY) {
        format!("{}d", seconds / DAY)
    } else if seconds >= HOUR && seconds.is_multiple_of(HOUR) {
        format!("{}h", seconds / HOUR)
    } else if seconds >= MINUTE && seconds.is_multiple_of(MINUTE) {
        format!("{}min", seconds / MINUTE)
    } else {
        format!("{seconds}s")
    }
}

fn weekday_name(weekday: u64) -> &'static str {
    match weekday {
        0 | 7 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        _ => "?",
    }
}

#[derive(Clone, Debug)]
pub struct Service {
    pub id: String,
    pub label: String,
    pub display_name: String,
    pub source: ServiceSource,
    pub scope: ServiceScope,
    pub domain: String,
    pub plist_path: Option<PathBuf>,
    pub config: LaunchConfig,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub status: ServiceStatus,
    pub enabled: Option<bool>,
    pub loaded: Option<bool>,
    pub brew_formula: Option<String>,
    pub brew_status: Option<String>,
    pub safety_level: SafetyLevel,
    pub elevation: ElevationNeeds,
    pub origin: Origin,
    pub health: Vec<String>,
}

impl Service {
    pub fn searchable_text(&self) -> String {
        let path = self
            .plist_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        format!(
            "{} {} {} {} {} {} {} {}",
            self.label,
            self.display_name,
            self.source,
            self.scope.label(),
            path,
            self.brew_formula.clone().unwrap_or_default(),
            self.origin.kind.label(),
            self.health.join(" ")
        )
        .to_lowercase()
    }
}

#[derive(Clone, Debug)]
pub struct Inventory {
    pub services: Vec<Service>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_schedule_describes_plain_times() {
        let schedule = CalendarSchedule {
            minute: Some(0),
            hour: Some(6),
            day: None,
            weekday: None,
            month: None,
        };

        assert_eq!(schedule.describe(), "06:00");
    }

    #[test]
    fn calendar_schedule_describes_calendar_dates_compactly() {
        let schedule = CalendarSchedule {
            minute: Some(30),
            hour: Some(9),
            day: None,
            weekday: Some(1),
            month: None,
        };

        assert_eq!(schedule.describe(), "Mon 09:30");
    }

    #[test]
    fn start_interval_uses_short_units() {
        assert_eq!(format_interval(300), "5min");
        assert_eq!(format_interval(3600), "1h");
        assert_eq!(format_interval(86_400), "1d");
        assert_eq!(format_interval(45), "45s");
    }

    #[test]
    fn launch_config_summarizes_interval_and_calendar_entries() {
        let config = LaunchConfig {
            program: None,
            arguments: Vec::new(),
            working_directory: None,
            stdout_path: None,
            stderr_path: None,
            run_at_load: None,
            keep_alive: None,
            start_interval: Some(3600),
            start_calendar_intervals: vec![
                CalendarSchedule {
                    minute: Some(0),
                    hour: Some(0),
                    day: None,
                    weekday: None,
                    month: None,
                },
                CalendarSchedule {
                    minute: Some(30),
                    hour: Some(12),
                    day: None,
                    weekday: None,
                    month: None,
                },
            ],
        };

        assert_eq!(config.schedule_summary(), "1h, 00:00, 12:30");
    }
}
