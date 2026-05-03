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
            Self::SystemDaemon => "daemon",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceStatus {
    Running,
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
        }
    }

    pub fn command_preview(&self) -> String {
        if !self.arguments.is_empty() {
            self.arguments.join(" ")
        } else {
            self.program.clone().unwrap_or_else(|| "-".to_string())
        }
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
            "{} {} {} {} {} {} {}",
            self.label,
            self.display_name,
            self.source,
            self.scope.label(),
            path,
            self.brew_formula.clone().unwrap_or_default(),
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
