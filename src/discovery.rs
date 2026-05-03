use crate::model::{
    CalendarSchedule, Inventory, LaunchConfig, SafetyLevel, Service, ServiceScope, ServiceSource,
    ServiceStatus,
};
use anyhow::{Context, Result};
use plist::Value;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct BrewService {
    name: String,
    status: String,
    user: Option<String>,
    file: Option<String>,
    exit_code: Option<i32>,
}

#[derive(Clone, Debug, Default)]
struct RuntimeState {
    pid: Option<u32>,
    exit_code: Option<i32>,
    loaded: bool,
}

pub fn load_inventory() -> Inventory {
    let mut warnings = Vec::new();
    let uid = current_uid();
    let mut services = BTreeMap::<String, Service>::new();

    for (dir, scope) in service_dirs() {
        if let Err(err) = read_plist_dir(&dir, scope, uid, &mut services) {
            warnings.push(format!("{}: {err}", dir.display()));
        }
    }

    let runtime = read_runtime(uid, &mut warnings);
    apply_runtime(&mut services, runtime);

    let disabled = read_disabled(uid, &mut warnings);
    apply_disabled(&mut services, disabled);

    apply_brew_services(uid, &mut services, &mut warnings);

    for service in services.values_mut() {
        finish_health(service);
    }

    Inventory {
        services: services.into_values().collect(),
        warnings,
    }
}

fn current_uid() -> u32 {
    let output = Command::new("id").arg("-u").output();
    output
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

fn service_dirs() -> Vec<(PathBuf, ServiceScope)> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push((home.join("Library/LaunchAgents"), ServiceScope::UserAgent));
    }
    dirs.push((
        PathBuf::from("/Library/LaunchAgents"),
        ServiceScope::GlobalAgent,
    ));
    dirs.push((
        PathBuf::from("/Library/LaunchDaemons"),
        ServiceScope::SystemDaemon,
    ));
    dirs.push((
        PathBuf::from("/System/Library/LaunchAgents"),
        ServiceScope::GlobalAgent,
    ));
    dirs.push((
        PathBuf::from("/System/Library/LaunchDaemons"),
        ServiceScope::SystemDaemon,
    ));
    dirs
}

fn read_plist_dir(
    dir: &Path,
    scope: ServiceScope,
    uid: u32,
    services: &mut BTreeMap<String, Service>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("plist") {
            continue;
        }

        match service_from_plist(&path, scope.clone(), uid) {
            Ok(service) => {
                services.insert(service.id.clone(), service);
            }
            Err(err) => {
                let id = format!("unreadable:{}", path.display());
                services.insert(
                    id.clone(),
                    Service {
                        id,
                        label: path
                            .file_stem()
                            .and_then(|name| name.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        display_name: path
                            .file_stem()
                            .and_then(|name| name.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        source: ServiceSource::Launchd,
                        scope: scope.clone(),
                        domain: scope.domain(uid),
                        plist_path: Some(path),
                        config: LaunchConfig::empty(),
                        pid: None,
                        exit_code: None,
                        status: ServiceStatus::Unknown,
                        enabled: None,
                        loaded: Some(false),
                        brew_formula: None,
                        brew_status: None,
                        safety_level: safety_for_scope(&scope, dir),
                        health: vec![format!("plist parse failed: {err}")],
                    },
                );
            }
        }
    }

    Ok(())
}

fn service_from_plist(path: &Path, scope: ServiceScope, uid: u32) -> Result<Service> {
    let value = Value::from_file(path).with_context(|| format!("parse {}", path.display()))?;
    let dict = value
        .as_dictionary()
        .with_context(|| format!("{} is not a plist dictionary", path.display()))?;

    let label = dict
        .get("Label")
        .and_then(Value::as_string)
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    let arguments = dict
        .get("ProgramArguments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_string)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let config = LaunchConfig {
        program: dict
            .get("Program")
            .and_then(Value::as_string)
            .map(str::to_string),
        arguments,
        working_directory: dict
            .get("WorkingDirectory")
            .and_then(Value::as_string)
            .map(str::to_string),
        stdout_path: dict
            .get("StandardOutPath")
            .and_then(Value::as_string)
            .map(str::to_string),
        stderr_path: dict
            .get("StandardErrorPath")
            .and_then(Value::as_string)
            .map(str::to_string),
        run_at_load: dict.get("RunAtLoad").and_then(Value::as_boolean),
        keep_alive: dict.get("KeepAlive").map(describe_plist_value),
        start_interval: dict
            .get("StartInterval")
            .and_then(Value::as_unsigned_integer),
        start_calendar_intervals: parse_start_calendar_intervals(dict.get("StartCalendarInterval")),
    };

    let domain = scope.domain(uid);
    let safety_level = safety_for_scope(
        &scope,
        path.parent()
            .unwrap_or_else(|| Path::new("/System/Library")),
    );
    let display_name = label
        .strip_prefix("homebrew.mxcl.")
        .unwrap_or(&label)
        .to_string();

    Ok(Service {
        id: format!("{domain}:{label}"),
        label,
        display_name,
        source: ServiceSource::Launchd,
        scope,
        domain,
        plist_path: Some(path.to_path_buf()),
        config,
        pid: None,
        exit_code: None,
        status: ServiceStatus::Unloaded,
        enabled: None,
        loaded: Some(false),
        brew_formula: None,
        brew_status: None,
        safety_level,
        health: Vec::new(),
    })
}

fn describe_plist_value(value: &Value) -> String {
    match value {
        Value::Boolean(value) => value.to_string(),
        Value::Dictionary(_) => "conditional".to_string(),
        Value::Array(_) => "array".to_string(),
        _ => "configured".to_string(),
    }
}

fn safety_for_scope(scope: &ServiceScope, dir: &Path) -> SafetyLevel {
    if dir.starts_with("/System/Library") {
        SafetyLevel::ReadonlySystem
    } else {
        match scope {
            ServiceScope::UserAgent => SafetyLevel::UserWritable,
            ServiceScope::GlobalAgent | ServiceScope::SystemDaemon => SafetyLevel::AdminRequired,
        }
    }
}

fn parse_start_calendar_intervals(value: Option<&Value>) -> Vec<CalendarSchedule> {
    match value {
        Some(Value::Dictionary(dict)) => calendar_schedule_from_dict(dict).into_iter().collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_dictionary)
            .filter_map(calendar_schedule_from_dict)
            .collect(),
        _ => Vec::new(),
    }
}

fn calendar_schedule_from_dict(dict: &plist::Dictionary) -> Option<CalendarSchedule> {
    let schedule = CalendarSchedule {
        minute: calendar_component(dict, "Minute"),
        hour: calendar_component(dict, "Hour"),
        day: calendar_component(dict, "Day"),
        weekday: calendar_component(dict, "Weekday"),
        month: calendar_component(dict, "Month"),
    };

    if schedule.minute.is_none()
        && schedule.hour.is_none()
        && schedule.day.is_none()
        && schedule.weekday.is_none()
        && schedule.month.is_none()
    {
        None
    } else {
        Some(schedule)
    }
}

fn calendar_component(dict: &plist::Dictionary, key: &str) -> Option<u64> {
    dict.get(key).and_then(Value::as_unsigned_integer)
}

fn read_runtime(uid: u32, warnings: &mut Vec<String>) -> HashMap<String, RuntimeState> {
    let mut runtime = HashMap::new();
    for domain in [
        format!("gui/{uid}"),
        format!("user/{uid}"),
        "system".to_string(),
    ] {
        let output = Command::new("launchctl").args(["print", &domain]).output();
        match output {
            Ok(output) if output.status.success() => {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    parse_launchctl_print(&domain, &text, &mut runtime);
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                warnings.push(format!("launchctl print {domain}: {}", err.trim()));
            }
            Err(err) => warnings.push(format!("launchctl print {domain}: {err}")),
        }
    }
    runtime
}

fn parse_launchctl_print(domain: &str, text: &str, runtime: &mut HashMap<String, RuntimeState>) {
    let mut in_services = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "services = {" {
            in_services = true;
            continue;
        }
        if in_services && trimmed == "}" {
            in_services = false;
            continue;
        }
        if !in_services || trimmed.is_empty() {
            continue;
        }

        let parts = trimmed.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }

        let label = parts[2..].join(" ");
        if label.starts_with("com.apple.xpc.")
            || label.starts_with("application.")
            || label.starts_with("UIKitApplication:")
        {
            continue;
        }

        let pid = parts[0].parse::<u32>().ok().filter(|pid| *pid > 0);
        let exit_code = parts[1].parse::<i32>().ok();

        runtime.insert(
            format!("{domain}:{label}"),
            RuntimeState {
                pid,
                exit_code,
                loaded: true,
            },
        );
    }
}

fn apply_runtime(services: &mut BTreeMap<String, Service>, runtime: HashMap<String, RuntimeState>) {
    let known_ids = services.keys().cloned().collect::<HashSet<_>>();

    for (id, state) in &runtime {
        if let Some(service) = services.get_mut(id) {
            service.pid = state.pid;
            service.exit_code = state.exit_code;
            service.loaded = Some(state.loaded);
            service.status = status_from_state(state);
        }
    }

    for (id, state) in runtime {
        if known_ids.contains(&id) {
            continue;
        }
        let Some((domain, label)) = id.split_once(':') else {
            continue;
        };
        let domain = domain.to_string();
        let label = label.to_string();
        let scope = if domain == "system" {
            ServiceScope::SystemDaemon
        } else {
            ServiceScope::UserAgent
        };
        services.insert(
            id.clone(),
            Service {
                id,
                label: label.clone(),
                display_name: label
                    .strip_prefix("homebrew.mxcl.")
                    .unwrap_or(&label)
                    .to_string(),
                source: ServiceSource::Launchd,
                scope,
                domain,
                plist_path: None,
                config: LaunchConfig::empty(),
                pid: state.pid,
                exit_code: state.exit_code,
                status: status_from_state(&state),
                enabled: None,
                loaded: Some(true),
                brew_formula: None,
                brew_status: None,
                safety_level: SafetyLevel::ProtectedVendor,
                health: vec!["loaded service has no discovered plist".to_string()],
            },
        );
    }
}

fn status_from_state(state: &RuntimeState) -> ServiceStatus {
    if state.pid.is_some() {
        ServiceStatus::Running
    } else if state.exit_code.is_some_and(|code| code != 0) {
        ServiceStatus::Failed
    } else {
        ServiceStatus::Stopped
    }
}

fn read_disabled(uid: u32, warnings: &mut Vec<String>) -> HashSet<String> {
    let mut disabled = HashSet::new();
    for domain in [
        format!("gui/{uid}"),
        format!("user/{uid}"),
        "system".to_string(),
    ] {
        let output = Command::new("launchctl")
            .args(["print-disabled", &domain])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    parse_disabled(&domain, &text, &mut disabled);
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                warnings.push(format!("launchctl print-disabled {domain}: {}", err.trim()));
            }
            Err(err) => warnings.push(format!("launchctl print-disabled {domain}: {err}")),
        }
    }
    disabled
}

fn parse_disabled(domain: &str, text: &str, disabled: &mut HashSet<String>) {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("=> true") {
            continue;
        }
        if let Some((label, _)) = trimmed.split_once("=>") {
            disabled.insert(format!("{}:{}", domain, label.trim().trim_matches('"')));
        }
    }
}

fn apply_disabled(services: &mut BTreeMap<String, Service>, disabled: HashSet<String>) {
    for service in services.values_mut() {
        if disabled.contains(&service.id) {
            service.enabled = Some(false);
            service.status = ServiceStatus::Disabled;
        } else {
            service.enabled = Some(true);
        }
    }
}

fn apply_brew_services(
    uid: u32,
    services: &mut BTreeMap<String, Service>,
    warnings: &mut Vec<String>,
) {
    let output = Command::new("brew")
        .args(["services", "list", "--json"])
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            warnings.push(format!("brew services list --json: {}", err.trim()));
            return;
        }
        Err(err) => {
            warnings.push(format!("brew services list --json: {err}"));
            return;
        }
    };

    let brew_services: Vec<BrewService> = match serde_json::from_slice(&output.stdout) {
        Ok(services) => services,
        Err(err) => {
            warnings.push(format!("brew services JSON parse failed: {err}"));
            return;
        }
    };

    for brew in brew_services {
        let label = label_for_brew(&brew);
        let domain = if brew.user.as_deref() == Some("root") {
            "system".to_string()
        } else {
            format!("gui/{uid}")
        };
        let id = format!("{domain}:{label}");
        let file = brew.file.as_ref().map(PathBuf::from);

        if let Some(service) = services.get_mut(&id) {
            service.source = ServiceSource::Both;
            service.display_name = brew.name.clone();
            service.brew_formula = Some(brew.name);
            service.brew_status = Some(brew.status.clone());
            service.exit_code = service.exit_code.or(brew.exit_code);
            if service.plist_path.is_none() {
                service.plist_path = file;
            }
            if service.status == ServiceStatus::Unloaded {
                service.status = status_from_brew(&brew.status, brew.exit_code);
            }
            continue;
        }

        let mut config = file
            .as_deref()
            .and_then(|path| service_from_plist(path, ServiceScope::UserAgent, uid).ok())
            .map(|service| service.config)
            .unwrap_or_else(LaunchConfig::empty);

        if config.program.is_none() && config.arguments.is_empty() {
            config.program = Some(format!("brew services start {}", brew.name));
        }

        services.insert(
            id.clone(),
            Service {
                id,
                label,
                display_name: brew.name.clone(),
                source: ServiceSource::Homebrew,
                scope: if domain == "system" {
                    ServiceScope::SystemDaemon
                } else {
                    ServiceScope::UserAgent
                },
                domain,
                plist_path: file,
                config,
                pid: None,
                exit_code: brew.exit_code,
                status: status_from_brew(&brew.status, brew.exit_code),
                enabled: None,
                loaded: None,
                brew_formula: Some(brew.name),
                brew_status: Some(brew.status),
                safety_level: SafetyLevel::UserWritable,
                health: Vec::new(),
            },
        );
    }
}

fn label_for_brew(brew: &BrewService) -> String {
    brew.file
        .as_deref()
        .and_then(|path| service_from_plist(Path::new(path), ServiceScope::UserAgent, 0).ok())
        .map(|service| service.label)
        .unwrap_or_else(|| format!("homebrew.mxcl.{}", brew.name))
}

fn status_from_brew(status: &str, exit_code: Option<i32>) -> ServiceStatus {
    match status {
        "started" => ServiceStatus::Running,
        "stopped" => ServiceStatus::Stopped,
        "error" => ServiceStatus::Failed,
        "none" => ServiceStatus::Unloaded,
        _ if exit_code.is_some_and(|code| code != 0) => ServiceStatus::Failed,
        _ => ServiceStatus::Unknown,
    }
}

fn finish_health(service: &mut Service) {
    if service.plist_path.is_none() {
        service
            .health
            .push("no source plist discovered for loaded service".to_string());
    }

    if let Some(path) = &service.plist_path
        && !path.exists()
    {
        service
            .health
            .push(format!("plist path does not exist: {}", path.display()));
    }

    if service.config.program.is_none() && service.config.arguments.is_empty() {
        service
            .health
            .push("no Program or ProgramArguments in parsed plist".to_string());
    }

    if let Some(program) = service
        .config
        .arguments
        .first()
        .or(service.config.program.as_ref())
        && program.starts_with('/')
        && !Path::new(program).exists()
    {
        service
            .health
            .push(format!("executable does not exist: {program}"));
    }
}
