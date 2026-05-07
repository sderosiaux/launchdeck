use crate::model::Service;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

const LOG_BIN: &str = "/usr/bin/log";
const MAX_LINES: usize = 500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogWindow {
    FiveMin,
    FifteenMin,
    OneHour,
}

impl LogWindow {
    pub fn label(self) -> &'static str {
        match self {
            Self::FiveMin => "5m",
            Self::FifteenMin => "15m",
            Self::OneHour => "1h",
        }
    }

    pub fn arg(self) -> &'static str {
        match self {
            Self::FiveMin => "5m",
            Self::FifteenMin => "15m",
            Self::OneHour => "1h",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::FiveMin => Self::FifteenMin,
            Self::FifteenMin => Self::OneHour,
            Self::OneHour => Self::FiveMin,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UnifiedLogQuery {
    pub label: String,
    pub executable: Option<String>,
    pub window: LogWindow,
}

#[derive(Clone, Debug)]
pub struct UnifiedLogResult {
    pub service_id: String,
    pub query: UnifiedLogQuery,
    pub elapsed_ms: u128,
    pub lines: Vec<String>,
    pub error: Option<String>,
}

pub fn build_query(service: &Service, window: LogWindow) -> Option<UnifiedLogQuery> {
    let label = sanitize(&service.label)?;
    let executable = derive_executable(service).and_then(|name| sanitize(&name));
    Some(UnifiedLogQuery {
        label,
        executable,
        window,
    })
}

pub fn derive_executable(service: &Service) -> Option<String> {
    let path = service
        .config
        .program
        .as_deref()
        .or_else(|| service.config.arguments.first().map(String::as_str))?;
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

pub fn build_predicate(query: &UnifiedLogQuery) -> String {
    let lifecycle = format!(
        r#"(processImagePath ENDSWITH "/launchd" AND eventMessage CONTAINS "{}")"#,
        query.label
    );
    match &query.executable {
        Some(exec) => format!(r#"{lifecycle} OR process == "{exec}""#),
        None => lifecycle,
    }
}

pub fn fetch(service_id: String, query: UnifiedLogQuery) -> UnifiedLogResult {
    let started = Instant::now();
    let predicate = build_predicate(&query);
    let output = Command::new(LOG_BIN)
        .args([
            "show",
            "--last",
            query.window.arg(),
            "--style",
            "compact",
            "--predicate",
            &predicate,
        ])
        .output();

    let mut lines = Vec::new();
    let mut error = None;
    match output {
        Ok(out) if out.status.success() => {
            let raw = String::from_utf8_lossy(&out.stdout);
            lines = parse_compact(&raw);
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            error = Some(format!(
                "log show exited with {}: {}",
                out.status,
                stderr.trim()
            ));
        }
        Err(err) => error = Some(format!("failed to spawn /usr/bin/log: {err}")),
    }

    UnifiedLogResult {
        service_id,
        query,
        elapsed_ms: started.elapsed().as_millis(),
        lines,
        error,
    }
}

fn parse_compact(raw: &str) -> Vec<String> {
    let collected: Vec<String> = raw
        .lines()
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("Timestamp"))
        .map(str::to_string)
        .collect();
    if collected.len() > MAX_LINES {
        collected[collected.len() - MAX_LINES..].to_vec()
    } else {
        collected
    }
}

fn sanitize(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let ok = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    if ok { Some(value.to_string()) } else { None }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::{
        LaunchConfig, SafetyLevel, Service, ServiceScope, ServiceSource, ServiceStatus,
    };

    fn sample_service() -> Service {
        let mut config = LaunchConfig::empty();
        config.program = Some("/opt/homebrew/opt/redis/bin/redis-server".to_string());
        Service {
            id: "homebrew.mxcl.redis".to_string(),
            label: "homebrew.mxcl.redis".to_string(),
            display_name: "redis".to_string(),
            source: ServiceSource::Homebrew,
            scope: ServiceScope::UserAgent,
            domain: "gui/501".to_string(),
            plist_path: None,
            config,
            pid: None,
            exit_code: None,
            status: ServiceStatus::Running,
            enabled: Some(true),
            loaded: Some(true),
            brew_formula: Some("redis".to_string()),
            brew_status: Some("started".to_string()),
            safety_level: SafetyLevel::UserWritable,
            health: Vec::new(),
        }
    }

    #[test]
    fn derives_executable_from_program() {
        let s = sample_service();
        assert_eq!(derive_executable(&s).as_deref(), Some("redis-server"));
    }

    #[test]
    fn derives_executable_from_first_argument() {
        let mut s = sample_service();
        s.config.program = None;
        s.config.arguments = vec!["/usr/local/bin/cool tool".to_string(), "--flag".to_string()];
        assert_eq!(derive_executable(&s).as_deref(), Some("cool tool"));
    }

    #[test]
    fn build_predicate_includes_lifecycle_and_process() {
        let q = UnifiedLogQuery {
            label: "homebrew.mxcl.redis".to_string(),
            executable: Some("redis-server".to_string()),
            window: LogWindow::FifteenMin,
        };
        let pred = build_predicate(&q);
        assert!(pred.contains(r#"eventMessage CONTAINS "homebrew.mxcl.redis""#));
        assert!(pred.contains(r#"process == "redis-server""#));
    }

    #[test]
    fn build_predicate_falls_back_to_lifecycle_only() {
        let q = UnifiedLogQuery {
            label: "com.example.job".to_string(),
            executable: None,
            window: LogWindow::FiveMin,
        };
        assert_eq!(
            build_predicate(&q),
            r#"(processImagePath ENDSWITH "/launchd" AND eventMessage CONTAINS "com.example.job")"#
        );
    }

    #[test]
    fn sanitize_rejects_quotes_and_unicode() {
        assert!(sanitize(r#"label" OR 1=1"#).is_none());
        assert!(sanitize("la bel").is_none());
        assert!(sanitize("com.apple.Finder").is_some());
        assert!(sanitize("redis-server").is_some());
        assert!(sanitize("foo+bar.baz_1").is_some());
    }

    #[test]
    fn build_query_drops_unsafe_executable_but_keeps_label() {
        let mut s = sample_service();
        s.config.program = Some("/usr/local/bin/has space".to_string());
        let q = build_query(&s, LogWindow::FiveMin).unwrap();
        assert_eq!(q.label, "homebrew.mxcl.redis");
        assert!(q.executable.is_none());
    }

    #[test]
    #[ignore = "hits /usr/bin/log; run with `cargo test -- --ignored`"]
    fn fetch_against_finder_returns_within_a_few_seconds() {
        let mut s = sample_service();
        s.label = "com.apple.Finder".to_string();
        s.config.program =
            Some("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder".to_string());
        let q = build_query(&s, LogWindow::FifteenMin).unwrap();
        let result = fetch(s.id.clone(), q);
        assert!(
            result.elapsed_ms < 5000,
            "fetch took {}ms",
            result.elapsed_ms
        );
        assert!(result.error.is_none(), "error: {:?}", result.error);
    }

    #[test]
    fn parse_compact_strips_header_and_caps() {
        let raw = "Timestamp               Ty Process[PID:TID]\n2026-05-06 a\n2026-05-06 b\n";
        assert_eq!(parse_compact(raw), vec!["2026-05-06 a", "2026-05-06 b"]);
    }
}
