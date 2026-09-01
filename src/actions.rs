use crate::model::{SafetyLevel, Service, ServiceSource, ServiceStatus};
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    Start,
    Stop,
    Restart,
    ToggleEnabled,
    ToggleRunAtLoad,
    EditPlist,
    Delete,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::ToggleEnabled => "toggle enabled",
            Self::ToggleRunAtLoad => "toggle RunAtLoad",
            Self::EditPlist => "edit plist",
            Self::Delete => "delete",
        }
    }
}

/// Whether the planned command has to run as root. Kept separate from the command
/// itself so the confirmation prompt can still show exactly what will run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Elevation {
    #[default]
    None,
    Sudo,
}

#[derive(Clone, Debug)]
pub struct ActionPlan {
    pub kind: ActionKind,
    pub service_name: String,
    pub command: Vec<String>,
    pub warning: String,
    pub blocked_reason: Option<String>,
    pub elevation: Elevation,
}

impl ActionPlan {
    pub fn command_display(&self) -> String {
        if self.command.is_empty() {
            "-".to_string()
        } else {
            effective_command(self).join(" ")
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked_reason.is_some()
    }

    pub fn needs_sudo(&self) -> bool {
        self.elevation == Elevation::Sudo
    }
}

#[derive(Debug)]
pub struct ActionResult {
    pub success: bool,
    pub message: String,
}

/// launchd scope and file ownership are independent axes, so elevation is decided
/// per action, not per service: `launchctl kickstart gui/501/<label>` on an agent
/// from `/Library/LaunchAgents` needs no privileges, while deleting that same
/// root-owned plist does.
fn elevation_for(service: &Service, kind: ActionKind) -> Elevation {
    let needs_root = match kind {
        ActionKind::Start | ActionKind::Stop | ActionKind::Restart | ActionKind::ToggleEnabled => {
            service.elevation.runtime
        }
        ActionKind::ToggleRunAtLoad => service.elevation.plist_write,
        ActionKind::Delete => service.elevation.plist_remove || service.elevation.runtime,
        // `open -t` runs as the user; the editor is what will fail to save, and it
        // says so far more clearly than we could.
        ActionKind::EditPlist => false,
    };

    if needs_root {
        Elevation::Sudo
    } else {
        Elevation::None
    }
}

pub fn plan(service: &Service, kind: ActionKind) -> ActionPlan {
    if matches!(service.safety_level, SafetyLevel::ReadonlySystem) {
        return blocked(
            service,
            kind,
            "system services under /System/Library are inspect-only",
        );
    }

    if matches!(service.safety_level, SafetyLevel::ProtectedVendor) {
        return blocked(
            service,
            kind,
            "vendor/runtime services are protected until an explicit allowlist exists",
        );
    }

    let elevation = elevation_for(service, kind);

    let plan = if matches!(
        service.source,
        ServiceSource::Homebrew | ServiceSource::Both
    ) {
        plan_brew(service, kind)
    } else {
        plan_launchd(service, kind)
    };

    ActionPlan { elevation, ..plan }
}

/// The argv actually spawned, sudo prefix included. This is what the confirmation
/// prompt shows, so it must stay identical to what `execute*` runs.
pub fn effective_command(plan: &ActionPlan) -> Vec<String> {
    match plan.elevation {
        Elevation::None => plan.command.clone(),
        Elevation::Sudo => {
            let mut command = vec!["sudo".to_string(), "--".to_string()];
            command.extend(plan.command.iter().cloned());
            command
        }
    }
}

/// Run an elevated plan with stdio inherited, so `sudo` can prompt on the real
/// terminal. The caller must have left the alternate screen and raw mode first;
/// the password never passes through this process.
pub fn execute_elevated(plan: &ActionPlan) -> ActionResult {
    if let Some(reason) = &plan.blocked_reason {
        return ActionResult {
            success: false,
            message: reason.clone(),
        };
    }

    let command = effective_command(plan);
    let Some((program, args)) = command.split_first() else {
        return ActionResult {
            success: false,
            message: "action has no command".to_string(),
        };
    };

    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => ActionResult {
            success: true,
            message: "command succeeded".to_string(),
        },
        // A cancelled password prompt is a normal outcome, not a failure to report
        // as a broken command.
        Ok(status) => ActionResult {
            success: false,
            message: format!("command exited with {status}"),
        },
        Err(err) => ActionResult {
            success: false,
            message: err.to_string(),
        },
    }
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
        ActionKind::ToggleRunAtLoad => {
            return blocked(
                service,
                kind,
                "Homebrew service plists are managed by brew services",
            );
        }
        ActionKind::EditPlist | ActionKind::Delete => {
            return blocked(
                service,
                kind,
                "Homebrew service plists are managed by brew services",
            );
        }
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
        elevation: Elevation::None,
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
        ActionKind::Stop => vec!["launchctl".to_string(), "bootout".to_string(), target],
        ActionKind::Restart => {
            if service.loaded == Some(false) {
                let Some(path) = &service.plist_path else {
                    return blocked(service, kind, "unloaded service has no plist to bootstrap");
                };
                vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    format!(
                        "launchctl bootstrap {} {} && launchctl kickstart -k {}",
                        shell_quote(&service.domain),
                        shell_quote(&path.display().to_string()),
                        shell_quote(&target),
                    ),
                ]
            } else {
                vec![
                    "launchctl".to_string(),
                    "kickstart".to_string(),
                    "-k".to_string(),
                    target,
                ]
            }
        }
        ActionKind::ToggleEnabled => {
            let subcommand = if service.enabled == Some(false) {
                "enable"
            } else {
                "disable"
            };
            vec!["launchctl".to_string(), subcommand.to_string(), target]
        }
        ActionKind::ToggleRunAtLoad => {
            let Some(path) = &service.plist_path else {
                return blocked(service, kind, "service has no plist to update");
            };
            let desired = if service.config.run_at_load == Some(true) {
                "false"
            } else {
                "true"
            };
            let path = shell_quote(&path.display().to_string());
            vec![
                "/bin/sh".to_string(),
                "-lc".to_string(),
                format!(
                    "if /usr/libexec/PlistBuddy -c 'Print :RunAtLoad' {path} >/dev/null 2>&1; then /usr/libexec/PlistBuddy -c 'Set :RunAtLoad {desired}' {path}; else /usr/libexec/PlistBuddy -c 'Add :RunAtLoad bool {desired}' {path}; fi && plutil -lint {path}",
                ),
            ]
        }
        ActionKind::EditPlist => {
            let Some(path) = &service.plist_path else {
                return blocked(service, kind, "service has no plist to edit");
            };
            vec![
                "open".to_string(),
                "-t".to_string(),
                path.display().to_string(),
            ]
        }
        ActionKind::Delete => {
            let Some(path) = &service.plist_path else {
                return blocked(service, kind, "service has no plist to delete");
            };
            let path_display = path.display().to_string();
            if service.loaded == Some(false) {
                vec!["rm".to_string(), "--".to_string(), path_display]
            } else {
                let path = shell_quote(&path_display);
                vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    format!(
                        "launchctl bootout {} >/dev/null 2>&1 && rm -- {path}",
                        shell_quote(&target),
                    ),
                ]
            }
        }
    };

    ActionPlan {
        kind,
        service_name: service.display_name.clone(),
        command,
        warning: warning_for_launchd(service, kind),
        blocked_reason: None,
        elevation: Elevation::None,
    }
}

fn warning_for_launchd(service: &Service, kind: ActionKind) -> String {
    match kind {
        ActionKind::Start if service.loaded == Some(false) => {
            "This will bootstrap the plist into the launchd domain.".to_string()
        }
        ActionKind::Start => "This will ask launchd to start the selected job.".to_string(),
        ActionKind::Stop => "This unloads the selected launchd job from its domain.".to_string(),
        ActionKind::Restart if service.loaded == Some(false) => {
            "This will bootstrap the plist, then kickstart the launchd job.".to_string()
        }
        ActionKind::Restart => "This kills and immediately restarts the launchd job.".to_string(),
        ActionKind::ToggleEnabled if service.status == ServiceStatus::Disabled => {
            "This will enable the launchd job in its domain.".to_string()
        }
        ActionKind::ToggleEnabled => "This will disable the launchd job in its domain.".to_string(),
        ActionKind::ToggleRunAtLoad if service.config.run_at_load == Some(true) => {
            "This will set RunAtLoad to false in the plist.".to_string()
        }
        ActionKind::ToggleRunAtLoad => "This will set RunAtLoad to true in the plist.".to_string(),
        ActionKind::EditPlist => {
            "This opens the plist in the default macOS text editor.".to_string()
        }
        ActionKind::Delete if service.loaded == Some(false) => {
            "This permanently removes the plist file.".to_string()
        }
        ActionKind::Delete => {
            "This tries to unload the launchd job, then permanently removes the plist file."
                .to_string()
        }
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn blocked(service: &Service, kind: ActionKind, reason: &str) -> ActionPlan {
    ActionPlan {
        kind,
        service_name: service.display_name.clone(),
        command: Vec::new(),
        warning: reason.to_string(),
        blocked_reason: Some(reason.to_string()),
        elevation: Elevation::None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ElevationNeeds, LaunchConfig, Origin, ServiceScope};
    use std::path::PathBuf;

    fn service(source: ServiceSource, run_at_load: Option<bool>) -> Service {
        let mut config = LaunchConfig::empty();
        config.run_at_load = run_at_load;

        Service {
            id: "gui/501/com.example.demo".to_string(),
            label: "com.example.demo".to_string(),
            display_name: "com.example.demo".to_string(),
            source,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: Some(PathBuf::from("/tmp/com.example.demo.plist")),
            config,
            pid: None,
            exit_code: None,
            status: ServiceStatus::Stopped,
            enabled: Some(true),
            loaded: Some(true),
            brew_formula: Some("demo".to_string()),
            brew_status: None,
            safety_level: SafetyLevel::UserWritable,
            elevation: ElevationNeeds::none(),
            origin: Origin::unknown(),
            health: Vec::new(),
        }
    }

    #[test]
    fn global_agent_start_needs_no_sudo_even_though_its_plist_is_root_owned() {
        let mut service = service(ServiceSource::Launchd, Some(false));
        service.scope = ServiceScope::GlobalAgent;
        service.safety_level = SafetyLevel::AdminRequired;
        service.plist_path = Some(PathBuf::from(
            "/Library/LaunchAgents/com.example.demo.plist",
        ));
        // Root owns the file, but the job still runs in the caller's gui domain.
        service.elevation = ElevationNeeds {
            runtime: false,
            plist_write: true,
            plist_remove: true,
        };

        let plan = plan(&service, ActionKind::Start);

        assert!(!plan.is_blocked());
        assert!(!plan.needs_sudo());
        assert_eq!(plan.command.first().map(String::as_str), Some("launchctl"));
    }

    #[test]
    fn deleting_a_root_owned_plist_is_planned_through_sudo() {
        let mut service = service(ServiceSource::Launchd, Some(false));
        service.elevation = ElevationNeeds {
            runtime: false,
            plist_write: true,
            plist_remove: true,
        };

        let plan = plan(&service, ActionKind::Delete);

        assert!(!plan.is_blocked());
        assert!(plan.needs_sudo());
        assert_eq!(
            effective_command(&plan).first().map(String::as_str),
            Some("sudo")
        );
        assert!(plan.command_display().starts_with("sudo -- "));
    }

    #[test]
    fn system_daemon_runtime_actions_need_sudo() {
        let mut service = service(ServiceSource::Launchd, Some(false));
        service.scope = ServiceScope::SystemDaemon;
        service.domain = "system".to_string();
        service.elevation = ElevationNeeds {
            runtime: true,
            plist_write: true,
            plist_remove: true,
        };

        let plan = plan(&service, ActionKind::Stop);

        assert!(!plan.is_blocked());
        assert!(plan.needs_sudo());
        assert_eq!(
            effective_command(&plan),
            vec![
                "sudo".to_string(),
                "--".to_string(),
                "launchctl".to_string(),
                "bootout".to_string(),
                "system/com.example.demo".to_string(),
            ]
        );
    }

    #[test]
    fn editing_a_plist_never_asks_for_sudo() {
        let mut service = service(ServiceSource::Launchd, Some(false));
        service.elevation = ElevationNeeds {
            runtime: true,
            plist_write: true,
            plist_remove: true,
        };

        assert!(!plan(&service, ActionKind::EditPlist).needs_sudo());
    }

    #[test]
    fn run_at_load_toggle_updates_launchd_plist() {
        let plan = plan(
            &service(ServiceSource::Launchd, Some(false)),
            ActionKind::ToggleRunAtLoad,
        );

        assert!(!plan.is_blocked());
        assert_eq!(plan.command.first().map(String::as_str), Some("/bin/sh"));
        assert_eq!(plan.command.get(1).map(String::as_str), Some("-lc"));
        assert!(
            plan.command
                .get(2)
                .is_some_and(|command| command.contains("Add :RunAtLoad bool true"))
        );
        assert!(
            plan.command
                .get(2)
                .is_some_and(|command| command.contains("plutil -lint"))
        );
    }

    #[test]
    fn run_at_load_toggle_is_blocked_for_brew_services() {
        let plan = plan(
            &service(ServiceSource::Homebrew, Some(false)),
            ActionKind::ToggleRunAtLoad,
        );

        assert!(plan.is_blocked());
        assert_eq!(
            plan.blocked_reason.as_deref(),
            Some("Homebrew service plists are managed by brew services")
        );
    }

    #[test]
    fn edit_plist_opens_launchd_plist_in_text_editor() {
        let plan = plan(
            &service(ServiceSource::Launchd, Some(false)),
            ActionKind::EditPlist,
        );

        assert!(!plan.is_blocked());
        assert_eq!(
            plan.command,
            vec![
                "open".to_string(),
                "-t".to_string(),
                "/tmp/com.example.demo.plist".to_string()
            ]
        );
    }

    #[test]
    fn stop_launchd_service_unloads_job() {
        let plan = plan(
            &service(ServiceSource::Launchd, Some(false)),
            ActionKind::Stop,
        );

        assert!(!plan.is_blocked());
        assert_eq!(
            plan.command,
            vec![
                "launchctl".to_string(),
                "bootout".to_string(),
                "gui/501/com.example.demo".to_string(),
            ]
        );
    }

    #[test]
    fn delete_loaded_service_unloads_before_removing_plist() {
        let plan = plan(
            &service(ServiceSource::Launchd, Some(false)),
            ActionKind::Delete,
        );

        assert!(!plan.is_blocked());
        assert_eq!(plan.command.first().map(String::as_str), Some("/bin/sh"));
        assert!(plan.command.get(2).is_some_and(|command| {
            command.contains("launchctl bootout 'gui/501/com.example.demo'")
        }));
        assert!(
            plan.command
                .get(2)
                .is_some_and(|command| command.contains("rm -- '/tmp/com.example.demo.plist'"))
        );
    }
}
