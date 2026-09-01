use crate::model::{
    CalendarSchedule, Confidence, ElevationNeeds, Inventory, LaunchConfig, Origin, Provenance,
    SafetyLevel, Service, ServiceScope, ServiceSource, ServiceStatus,
};
use anyhow::{Context, Result};
use plist::Value;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize)]
struct BrewService {
    name: String,
    status: String,
    user: Option<String>,
    file: Option<String>,
    exit_code: Option<i32>,
}

/// `brew services list --json` costs ~0.9s of the ~1.1s inventory pass, and brew
/// state only changes when someone runs brew. Polling it at the launchctl cadence
/// burns battery for nothing, so it gets its own TTL and is bypassed after any
/// action the user just confirmed.
const BREW_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct BrewSnapshot {
    fetched: Instant,
    services: Vec<BrewService>,
}

pub type BrewCache = Arc<Mutex<Option<BrewSnapshot>>>;

pub fn new_brew_cache() -> BrewCache {
    Arc::new(Mutex::new(None))
}

#[derive(Clone, Debug, Default)]
struct RuntimeState {
    pid: Option<u32>,
    exit_code: Option<i32>,
    loaded: bool,
}

pub fn load_inventory() -> Inventory {
    load_inventory_with(None, true)
}

/// `cache` reuses a recent `brew services` snapshot; `force_brew` bypasses it when
/// the user just ran an action and expects the new state immediately.
pub fn load_inventory_with(cache: Option<&BrewCache>, force_brew: bool) -> Inventory {
    let mut warnings = Vec::new();
    let uid = current_uid();
    let mut services = BTreeMap::<String, Service>::new();

    for (dir, scope) in service_dirs() {
        if let Err(err) = read_plist_dir(&dir, scope, uid, &mut services) {
            warnings.push(format!("{}: {err}", dir.display()));
        }
    }

    let runtime = read_runtime(uid, &mut warnings);
    apply_runtime(&mut services, runtime, uid);

    let disabled = read_disabled(uid, &mut warnings);
    apply_disabled(&mut services, disabled);

    apply_brew_services(uid, &mut services, &mut warnings, cache, force_brew);

    for service in services.values_mut() {
        service.origin = classify_origin(service);
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
                let elevation = elevation_for(&scope.domain(uid), Some(&path), uid);
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
                        elevation,
                        origin: Origin::unknown(),
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
    let domain_for_elevation = domain.clone();
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
        elevation: elevation_for(&domain_for_elevation, Some(path), uid),
        origin: Origin::unknown(),
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

/// Best-effort write check without pulling in libc. Supplementary groups are not
/// resolved, so a group-writable root-owned file reads as "needs root". That errs
/// toward asking for elevation we did not strictly need, never toward skipping it.
fn is_writable(path: &Path, uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let Ok(meta) = path.metadata() else {
        return false;
    };
    let mode = meta.mode();
    if meta.uid() == uid {
        mode & 0o200 != 0
    } else {
        mode & 0o002 != 0
    }
}

fn elevation_for(domain: &str, plist_path: Option<&Path>, uid: u32) -> ElevationNeeds {
    ElevationNeeds {
        runtime: domain == "system" && uid != 0,
        plist_write: !plist_path.is_some_and(|path| is_writable(path, uid)),
        plist_remove: !plist_path
            .and_then(Path::parent)
            .is_some_and(|dir| is_writable(dir, uid)),
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

fn apply_runtime(
    services: &mut BTreeMap<String, Service>,
    runtime: HashMap<String, RuntimeState>,
    uid: u32,
) {
    let known_ids = services.keys().cloned().collect::<HashSet<_>>();

    for (id, state) in &runtime {
        if let Some(service) = services.get_mut(id) {
            service.pid = state.pid;
            service.exit_code = state.exit_code;
            service.loaded = Some(state.loaded);
            service.status = status_from_state(state, &service.config);
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
        let elevation = elevation_for(&domain, None, uid);
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
                status: status_from_state(&state, &LaunchConfig::empty()),
                enabled: None,
                loaded: Some(true),
                brew_formula: None,
                brew_status: None,
                safety_level: SafetyLevel::ProtectedVendor,
                elevation,
                origin: Origin::unknown(),
                health: Vec::new(),
            },
        );
    }
}

fn status_from_state(state: &RuntimeState, config: &LaunchConfig) -> ServiceStatus {
    if state.pid.is_some() {
        ServiceStatus::Running
    } else if state.exit_code.is_some_and(|code| code != 0) {
        ServiceStatus::Failed
    } else if has_schedule(config) {
        ServiceStatus::Scheduled
    } else {
        ServiceStatus::Stopped
    }
}

fn has_schedule(config: &LaunchConfig) -> bool {
    config.start_interval.is_some() || !config.start_calendar_intervals.is_empty()
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
        let Some((label, state)) = line.trim().split_once("=>") else {
            continue;
        };
        // macOS <= 12 prints `=> true`/`=> false`, macOS >= 13 prints `=> disabled`/`=> enabled`.
        if !matches!(state.trim(), "true" | "disabled") {
            continue;
        }
        disabled.insert(format!("{}:{}", domain, label.trim().trim_matches('"')));
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

fn read_brew_services(
    warnings: &mut Vec<String>,
    cache: Option<&BrewCache>,
    force: bool,
) -> Option<Vec<BrewService>> {
    if !force
        && let Some(cache) = cache
        && let Ok(guard) = cache.lock()
        && let Some(snapshot) = guard.as_ref()
        && snapshot.fetched.elapsed() < BREW_TTL
    {
        return Some(snapshot.services.clone());
    }

    let output = Command::new("brew")
        .args(["services", "list", "--json"])
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            warnings.push(format!("brew services list --json: {}", err.trim()));
            return None;
        }
        Err(err) => {
            warnings.push(format!("brew services list --json: {err}"));
            return None;
        }
    };

    let brew_services: Vec<BrewService> = match serde_json::from_slice(&output.stdout) {
        Ok(services) => services,
        Err(err) => {
            warnings.push(format!("brew services JSON parse failed: {err}"));
            return None;
        }
    };

    if let Some(cache) = cache
        && let Ok(mut guard) = cache.lock()
    {
        *guard = Some(BrewSnapshot {
            fetched: Instant::now(),
            services: brew_services.clone(),
        });
    }

    Some(brew_services)
}

fn apply_brew_services(
    uid: u32,
    services: &mut BTreeMap<String, Service>,
    warnings: &mut Vec<String>,
    cache: Option<&BrewCache>,
    force: bool,
) {
    let Some(brew_services) = read_brew_services(warnings, cache, force) else {
        return;
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
                // The plist only just became known, so the earlier elevation guess
                // (computed without a path) is stale.
                service.elevation =
                    elevation_for(&service.domain, service.plist_path.as_deref(), uid);
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

        let elevation = elevation_for(&domain, file.as_deref(), uid);
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
                elevation,
                origin: Origin::unknown(),
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

/// Classify who installed the job, from signals already in memory. Ordering is
/// the whole design: a nix-darwin daemon lives in `/Library/LaunchDaemons` like
/// any vendor helper, so the `/nix/store` command path has to win over the
/// directory it sits in.
fn classify_origin(service: &Service) -> Origin {
    let plist = service
        .plist_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let command = service
        .config
        .program
        .as_deref()
        .or_else(|| service.config.arguments.first().map(String::as_str))
        .unwrap_or_default();

    let origin = |kind: Provenance, confidence: Confidence, evidence: &str| Origin {
        kind,
        confidence,
        evidence: vec![evidence.to_string()],
    };

    if let Some(formula) = &service.brew_formula {
        return origin(
            Provenance::Homebrew,
            Confidence::High,
            &format!("brew services knows formula `{formula}`"),
        );
    }
    if service.label.starts_with("homebrew.mxcl.") {
        return origin(
            Provenance::Homebrew,
            Confidence::High,
            "label uses the homebrew.mxcl. prefix",
        );
    }
    if plist.contains("/Cellar/") || command.contains("/Cellar/") {
        return origin(
            Provenance::Homebrew,
            Confidence::Medium,
            "path points into the Homebrew Cellar",
        );
    }

    if command.contains("/nix/store/") {
        return origin(
            Provenance::Nix,
            Confidence::High,
            "command resolves into /nix/store",
        );
    }
    if command.contains("/mise") || service.label.contains("mise") {
        return origin(
            Provenance::Mise,
            Confidence::Medium,
            "command or label references mise",
        );
    }

    if plist.starts_with("/System/Library") {
        return origin(
            Provenance::System,
            Confidence::High,
            "plist lives under /System/Library",
        );
    }
    if service.label.starts_with("com.apple.") {
        return origin(
            Provenance::System,
            Confidence::High,
            "label uses the com.apple. prefix",
        );
    }

    if service.plist_path.is_none() {
        return origin(
            Provenance::RuntimeOnly,
            Confidence::High,
            "loaded in launchd with no plist on disk",
        );
    }

    if looks_like_vendor_command(command) {
        return origin(
            Provenance::VendorApp,
            Confidence::Medium,
            "command lives inside an app bundle",
        );
    }

    if let Some(home) = dirs::home_dir()
        && plist.starts_with(&home.join("Library/LaunchAgents").display().to_string())
    {
        return origin(
            Provenance::UserPlist,
            Confidence::High,
            "plist lives in ~/Library/LaunchAgents",
        );
    }

    if plist.starts_with("/Library/Launch") {
        return origin(
            Provenance::VendorApp,
            Confidence::Guess,
            "machine-wide plist, no installer signature",
        );
    }

    Origin::unknown()
}

fn looks_like_vendor_command(command: &str) -> bool {
    command.contains(".app/Contents/")
        || command.contains("/Library/Application Support/")
        || command.contains("/Library/PrivilegedHelperTools/")
}

fn finish_health(service: &mut Service) {
    if service.plist_path.is_none() {
        push_unique_health(
            service,
            "no source plist discovered for loaded service".to_string(),
        );
        return;
    }

    if service.config.program.is_none() && service.config.arguments.is_empty() {
        push_unique_health(
            service,
            "no Program or ProgramArguments in parsed plist".to_string(),
        );
    }

    if let Some(path) = &service.plist_path
        && !path.exists()
    {
        push_unique_health(
            service,
            format!("plist path does not exist: {}", path.display()),
        );
    }

    if let Some(program) = service
        .config
        .arguments
        .first()
        .or(service.config.program.as_ref())
        && program.starts_with('/')
        && !Path::new(program).exists()
    {
        push_unique_health(service, format!("executable does not exist: {program}"));
    }
}

fn push_unique_health(service: &mut Service, warning: String) {
    if !service.health.contains(&warning) {
        service.health.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service_with_plist(config: LaunchConfig) -> Service {
        Service {
            id: "gui/501:com.example.demo".to_string(),
            label: "com.example.demo".to_string(),
            display_name: "com.example.demo".to_string(),
            source: ServiceSource::Launchd,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: Some(PathBuf::from("/tmp/com.example.demo.plist")),
            config,
            pid: None,
            exit_code: None,
            status: ServiceStatus::Unloaded,
            enabled: Some(true),
            loaded: Some(false),
            brew_formula: None,
            brew_status: None,
            safety_level: SafetyLevel::UserWritable,
            elevation: ElevationNeeds::none(),
            origin: Origin::unknown(),
            health: Vec::new(),
        }
    }

    fn runtime_only_service() -> Service {
        Service {
            id: "gui/501:com.example.runtime".to_string(),
            label: "com.example.runtime".to_string(),
            display_name: "com.example.runtime".to_string(),
            source: ServiceSource::Launchd,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: None,
            config: LaunchConfig::empty(),
            pid: Some(123),
            exit_code: None,
            status: ServiceStatus::Running,
            enabled: None,
            loaded: Some(true),
            brew_formula: None,
            brew_status: None,
            safety_level: SafetyLevel::ProtectedVendor,
            elevation: ElevationNeeds::none(),
            origin: Origin::unknown(),
            health: Vec::new(),
        }
    }

    #[test]
    fn runtime_only_service_health_does_not_claim_parsed_plist() {
        let mut service = runtime_only_service();

        finish_health(&mut service);

        assert_eq!(
            service.health,
            vec!["no source plist discovered for loaded service".to_string()]
        );
    }

    #[test]
    fn parsed_plist_without_program_gets_program_warning() {
        let mut service = service_with_plist(LaunchConfig::empty());

        finish_health(&mut service);

        assert!(
            service
                .health
                .iter()
                .any(|warning| { warning == "no Program or ProgramArguments in parsed plist" })
        );
    }

    #[test]
    fn loaded_scheduled_service_is_not_reported_as_stopped() {
        let mut config = LaunchConfig::empty();
        config.start_interval = Some(300);

        let state = RuntimeState {
            pid: None,
            exit_code: Some(0),
            loaded: true,
        };

        assert_eq!(status_from_state(&state, &config), ServiceStatus::Scheduled);
    }

    fn service_with_command(command: &str, plist: Option<&str>) -> Service {
        let mut service = service_with_plist(LaunchConfig::empty());
        service.config.program = Some(command.to_string());
        service.plist_path = plist.map(PathBuf::from);
        service
    }

    #[test]
    fn nix_command_wins_over_the_system_directory_it_lives_in() {
        let service = service_with_command(
            "/nix/store/abc-my-daemon/bin/my-daemon",
            Some("/Library/LaunchDaemons/org.nixos.my-daemon.plist"),
        );

        let origin = classify_origin(&service);

        assert_eq!(origin.kind, Provenance::Nix);
        assert_eq!(origin.confidence, Confidence::High);
    }

    #[test]
    fn brew_formula_beats_every_path_heuristic() {
        let mut service = service_with_command("/opt/homebrew/opt/redis/bin/redis-server", None);
        service.brew_formula = Some("redis".to_string());

        assert_eq!(classify_origin(&service).kind, Provenance::Homebrew);
    }

    #[test]
    fn vendor_helper_in_user_agents_is_not_reported_as_a_hand_written_plist() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let plist = home.join("Library/LaunchAgents/com.vendor.helper.plist");
        let service = service_with_command(
            "/Applications/Vendor.app/Contents/MacOS/helper",
            plist.to_str(),
        );

        assert_eq!(classify_origin(&service).kind, Provenance::VendorApp);
    }

    #[test]
    fn loaded_job_without_a_plist_is_runtime_only() {
        let mut service = runtime_only_service();
        service.config.program = Some("/usr/bin/true".to_string());

        assert_eq!(classify_origin(&service).kind, Provenance::RuntimeOnly);
    }

    #[test]
    fn root_owned_agent_needs_sudo_for_the_plist_but_not_for_launchctl() {
        // /Library/LaunchAgents is root-owned and not world-writable, yet its jobs
        // live in the caller's gui domain.
        let needs = elevation_for("gui/501", Some(Path::new("/Library/LaunchAgents")), 501);

        assert!(!needs.runtime);
        assert!(needs.plist_write);
    }

    #[test]
    fn system_domain_needs_sudo_at_runtime() {
        let needs = elevation_for("system", None, 501);

        assert!(needs.runtime);
    }

    #[test]
    fn root_needs_no_elevation_anywhere() {
        let needs = elevation_for("system", Some(Path::new("/Library/LaunchDaemons")), 0);

        assert_eq!(needs, ElevationNeeds::none());
    }

    #[test]
    fn a_fresh_brew_snapshot_is_reused_instead_of_shelling_out() {
        let cache = new_brew_cache();
        let sentinel = BrewService {
            name: "cached-sentinel".to_string(),
            status: "started".to_string(),
            user: None,
            file: None,
            exit_code: None,
        };
        let Ok(mut guard) = cache.lock() else {
            panic!("cache lock");
        };
        *guard = Some(BrewSnapshot {
            fetched: Instant::now(),
            services: vec![sentinel],
        });
        drop(guard);

        let mut warnings = Vec::new();
        let services = read_brew_services(&mut warnings, Some(&cache), false);

        // Getting the sentinel back proves `brew` was never spawned.
        assert_eq!(
            services.map(|list| list.into_iter().map(|brew| brew.name).collect::<Vec<_>>()),
            Some(vec!["cached-sentinel".to_string()])
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn an_expired_brew_snapshot_is_not_reused() {
        let cache = new_brew_cache();
        let Ok(mut guard) = cache.lock() else {
            panic!("cache lock");
        };
        let Some(stale) = Instant::now().checked_sub(BREW_TTL * 2) else {
            return;
        };
        *guard = Some(BrewSnapshot {
            fetched: stale,
            services: vec![BrewService {
                name: "cached-sentinel".to_string(),
                status: "started".to_string(),
                user: None,
                file: None,
                exit_code: None,
            }],
        });
        drop(guard);

        let mut warnings = Vec::new();
        let services = read_brew_services(&mut warnings, Some(&cache), false);

        let names = services
            .unwrap_or_default()
            .into_iter()
            .map(|brew| brew.name)
            .collect::<Vec<_>>();
        assert!(!names.contains(&"cached-sentinel".to_string()));
    }

    #[test]
    fn disabled_parses_modern_macos_wording() {
        let text = r#"
	disabled services = {
		"com.docker.helper" => enabled
		"com.example.off" => disabled
	}
"#;
        let mut disabled = HashSet::new();
        parse_disabled("gui/501", text, &mut disabled);

        assert!(disabled.contains("gui/501:com.example.off"));
        assert!(!disabled.contains("gui/501:com.docker.helper"));
    }

    #[test]
    fn disabled_still_parses_legacy_boolean_wording() {
        let text = "\t\t\"com.example.off\" => true\n\t\t\"com.example.on\" => false\n";
        let mut disabled = HashSet::new();
        parse_disabled("system", text, &mut disabled);

        assert_eq!(
            disabled.into_iter().collect::<Vec<_>>(),
            vec!["system:com.example.off".to_string()]
        );
    }

    #[test]
    fn unloaded_scheduled_service_stays_unloaded_until_bootstrapped() {
        let mut service = service_with_plist(LaunchConfig::empty());
        service.config.start_interval = Some(300);

        finish_health(&mut service);

        assert_eq!(service.status, ServiceStatus::Unloaded);
    }
}
