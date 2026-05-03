use crate::model::{SafetyLevel, Service, ServiceSource, ServiceStatus};
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    Start,
    Stop,
    Restart,
    ToggleEnabled,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::ToggleEnabled => "toggle enabled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ActionPlan {
    pub kind: ActionKind,
    pub service_name: String,
    pub command: Vec<String>,
    pub warning: String,
    pub blocked_reason: Option<String>,
}

impl ActionPlan {
    pub fn command_display(&self) -> String {
        if self.command.is_empty() {
            "-".to_string()
        } else {
            self.command.join(" ")
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked_reason.is_some()
    }
}

#[derive(Debug)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
}

pub fn plan(service: &Service, kind: ActionKind) -> ActionPlan {
    if matches!(service.safety_level, SafetyLevel::ReadonlySystem) {
        return blocked(
            service,
            kind,
            "system services under /System/Library are inspect-only",
        );
    }

    if matches!(service.safety_level, SafetyLevel::AdminRequired) {
        return blocked(
            service,
            kind,
            "this service needs admin privileges; sudo execution is not implemented yet",
        );
    }

    if matches!(service.safety_level, SafetyLevel::ProtectedVendor) {
        return blocked(
            service,
            kind,
            "vendor/runtime services are protected until an explicit allowlist exists",
        );
    }

    if matches!(
        service.source,
        ServiceSource::Homebrew | ServiceSource::Both
    ) {
        return plan_brew(service, kind);
    }

    plan_launchd(service, kind)
}

pub fn execute(plan: &ActionPlan) -> ActionResult {
    if let Some(reason) = &plan.blocked_reason {
        return ActionResult {
            success: false,
            message: reason.clone(),
        };
    }

    let Some(program) = plan.command.first() else {
        return ActionResult {
            success: false,
            message: "action has no command".to_string(),
        };
    };

    let output = Command::new(program).args(&plan.command[1..]).output();
    match output {
        Ok(output) if output.status.success() => ActionResult {
            success: true,
            message: compact_output(&output.stdout, "command succeeded"),
        },
        Ok(output) => ActionResult {
            success: false,
            message: compact_output(&output.stderr, "command failed"),
        },
        Err(err) => ActionResult {
            success: false,
            message: err.to_string(),
        },
    }
}

fn plan_brew(service: &Service, kind: ActionKind) -> ActionPlan {
    let Some(formula) = &service.brew_formula else {
        return blocked(service, kind, "Homebrew service has no formula name");
    };

    let subcommand = match kind {
        ActionKind::Start => "start",
        ActionKind::Stop => "stop",
        ActionKind::Restart => "restart",
        ActionKind::ToggleEnabled => {
            return blocked(
                service,
                kind,
                "Homebrew has no enable-only toggle; use start, stop, or restart",
            );
        }
    };

    ActionPlan {
        kind,
        service_name: service.display_name.clone(),
        command: vec![
            "brew".to_string(),
            "services".to_string(),
            subcommand.to_string(),
            formula.clone(),
        ],
        warning: "Homebrew will update the service registration for this formula.".to_string(),
        blocked_reason: None,
    }
}

fn plan_launchd(service: &Service, kind: ActionKind) -> ActionPlan {
    let target = format!("{}/{}", service.domain, service.label);
    let command = match kind {
        ActionKind::Start => {
            if service.loaded == Some(false) {
                let Some(path) = &service.plist_path else {
                    return blocked(service, kind, "unloaded service has no plist to bootstrap");
                };
                vec![
                    "launchctl".to_string(),
                    "bootstrap".to_string(),
                    service.domain.clone(),
                    path.display().to_string(),
                ]
            } else {
                vec!["launchctl".to_string(), "kickstart".to_string(), target]
            }
        }
        ActionKind::Stop => vec![
            "launchctl".to_string(),
            "kill".to_string(),
            "TERM".to_string(),
            target,
        ],
        ActionKind::Restart => vec![
            "launchctl".to_string(),
            "kickstart".to_string(),
            "-k".to_string(),
            target,
        ],
        ActionKind::ToggleEnabled => {
            let subcommand = if service.enabled == Some(false) {
                "enable"
            } else {
                "disable"
            };
            vec!["launchctl".to_string(), subcommand.to_string(), target]
        }
    };

    ActionPlan {
        kind,
        service_name: service.display_name.clone(),
        command,
        warning: warning_for_launchd(service, kind),
        blocked_reason: None,
    }
}

fn warning_for_launchd(service: &Service, kind: ActionKind) -> String {
    match kind {
        ActionKind::Start if service.loaded == Some(false) => {
            "This will bootstrap the plist into the launchd domain.".to_string()
        }
        ActionKind::Start => "This will ask launchd to start the selected job.".to_string(),
        ActionKind::Stop => "This sends TERM to the selected launchd job.".to_string(),
        ActionKind::Restart => "This kills and immediately restarts the launchd job.".to_string(),
        ActionKind::ToggleEnabled if service.status == ServiceStatus::Disabled => {
            "This will enable the launchd job in its domain.".to_string()
        }
        ActionKind::ToggleEnabled => "This will disable the launchd job in its domain.".to_string(),
    }
}

fn blocked(service: &Service, kind: ActionKind, reason: &str) -> ActionPlan {
    ActionPlan {
        kind,
        service_name: service.display_name.clone(),
        command: Vec::new(),
        warning: reason.to_string(),
        blocked_reason: Some(reason.to_string()),
    }
}

fn compact_output(bytes: &[u8], fallback: &str) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    if text.is_empty() {
        fallback.to_string()
    } else {
        text.lines().take(3).collect::<Vec<_>>().join(" | ")
    }
}
