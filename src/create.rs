use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateField {
    Label,
    Command,
    Arguments,
    WorkingDirectory,
    Environment,
    StartInterval,
    Stdout,
    Stderr,
    RunAtLoad,
    KeepAlive,
    BootstrapNow,
    StartNow,
}

impl CreateField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Label => "Label",
            Self::Command => "Command",
            Self::Arguments => "Arguments",
            Self::WorkingDirectory => "Working directory",
            Self::Environment => "Environment",
            Self::StartInterval => "Start interval",
            Self::Stdout => "Stdout path",
            Self::Stderr => "Stderr path",
            Self::RunAtLoad => "Run at load",
            Self::KeepAlive => "Keep alive",
            Self::BootstrapNow => "Bootstrap now",
            Self::StartNow => "Start now",
        }
    }

    fn editable(self) -> bool {
        !matches!(
            self,
            Self::RunAtLoad | Self::KeepAlive | Self::BootstrapNow | Self::StartNow
        )
    }
}

const FIELDS: [CreateField; 12] = [
    CreateField::Label,
    CreateField::Command,
    CreateField::Arguments,
    CreateField::WorkingDirectory,
    CreateField::Environment,
    CreateField::StartInterval,
    CreateField::Stdout,
    CreateField::Stderr,
    CreateField::RunAtLoad,
    CreateField::KeepAlive,
    CreateField::BootstrapNow,
    CreateField::StartNow,
];

#[derive(Clone, Debug)]
pub struct CreateServiceForm {
    pub label: String,
    pub command: String,
    pub arguments: String,
    pub working_directory: String,
    pub environment: String,
    pub start_interval: String,
    pub stdout_path: String,
    pub stderr_path: String,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub bootstrap_now: bool,
    pub start_now: bool,
    pub selected: usize,
}

#[derive(Clone, Debug)]
pub struct CreateOutcome {
    pub path: PathBuf,
    pub steps: Vec<String>,
}

impl CreateOutcome {
    pub fn status_message(&self) -> String {
        if self.steps.is_empty() {
            format!("created {}", self.path.display())
        } else {
            format!("created {}; {}", self.path.display(), self.steps.join("; "))
        }
    }
}

impl CreateServiceForm {
    pub fn new() -> Self {
        Self {
            label: String::new(),
            command: String::new(),
            arguments: String::new(),
            working_directory: String::new(),
            environment: String::new(),
            start_interval: String::new(),
            stdout_path: String::new(),
            stderr_path: String::new(),
            run_at_load: false,
            keep_alive: false,
            bootstrap_now: false,
            start_now: false,
            selected: 0,
        }
    }

    pub fn fields() -> &'static [CreateField] {
        &FIELDS
    }

    pub fn current_field(&self) -> CreateField {
        FIELDS[self.selected]
    }

    pub fn value_for(&self, field: CreateField) -> String {
        match field {
            CreateField::Label => required_marker(&self.label),
            CreateField::Command => required_marker(&self.command),
            CreateField::Arguments => empty_marker(&self.arguments),
            CreateField::WorkingDirectory => empty_marker(&self.working_directory),
            CreateField::Environment => empty_marker(&self.environment),
            CreateField::StartInterval => empty_marker(&self.start_interval),
            CreateField::Stdout => empty_marker(&self.stdout_path),
            CreateField::Stderr => empty_marker(&self.stderr_path),
            CreateField::RunAtLoad => bool_marker(self.run_at_load),
            CreateField::KeepAlive => bool_marker(self.keep_alive),
            CreateField::BootstrapNow => bool_marker(self.bootstrap_now),
            CreateField::StartNow => bool_marker(self.start_now),
        }
    }

    pub fn next(&mut self) {
        self.selected = (self.selected + 1) % FIELDS.len();
    }

    pub fn previous(&mut self) {
        self.selected = if self.selected == 0 {
            FIELDS.len() - 1
        } else {
            self.selected - 1
        };
    }

    pub fn insert(&mut self, value: char) {
        if !self.current_field().editable() {
            return;
        }
        self.current_value_mut().push(value);
    }

    pub fn backspace(&mut self) {
        if !self.current_field().editable() {
            return;
        }
        self.current_value_mut().pop();
    }

    pub fn toggle(&mut self) {
        match self.current_field() {
            CreateField::RunAtLoad => self.run_at_load = !self.run_at_load,
            CreateField::KeepAlive => self.keep_alive = !self.keep_alive,
            CreateField::BootstrapNow => {
                self.bootstrap_now = !self.bootstrap_now;
                if !self.bootstrap_now {
                    self.start_now = false;
                }
            }
            CreateField::StartNow => {
                self.start_now = !self.start_now;
                if self.start_now {
                    self.bootstrap_now = true;
                }
            }
            _ => {}
        }
    }

    pub fn target_path(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().context("home directory not found")?;
        Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", self.label.trim())))
    }

    fn current_value_mut(&mut self) -> &mut String {
        match self.current_field() {
            CreateField::Label => &mut self.label,
            CreateField::Command => &mut self.command,
            CreateField::Arguments => &mut self.arguments,
            CreateField::WorkingDirectory => &mut self.working_directory,
            CreateField::Environment => &mut self.environment,
            CreateField::StartInterval => &mut self.start_interval,
            CreateField::Stdout => &mut self.stdout_path,
            CreateField::Stderr => &mut self.stderr_path,
            CreateField::RunAtLoad
            | CreateField::KeepAlive
            | CreateField::BootstrapNow
            | CreateField::StartNow => unreachable!(),
        }
    }
}

pub fn write_user_agent(form: &CreateServiceForm) -> Result<CreateOutcome> {
    validate(form)?;

    let path = form.target_path()?;
    if path.exists() {
        bail!("plist already exists: {}", path.display());
    }

    let parent = path
        .parent()
        .context("plist target has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let mut dict = Dictionary::new();
    dict.insert("Label".into(), Value::String(form.label.trim().to_string()));
    dict.insert(
        "ProgramArguments".into(),
        Value::Array(program_arguments(form)?),
    );
    dict.insert("RunAtLoad".into(), Value::Boolean(form.run_at_load));
    dict.insert("KeepAlive".into(), Value::Boolean(form.keep_alive));

    insert_optional_string(&mut dict, "WorkingDirectory", &form.working_directory);
    insert_optional_string(&mut dict, "StandardOutPath", &form.stdout_path);
    insert_optional_string(&mut dict, "StandardErrorPath", &form.stderr_path);
    insert_environment(&mut dict, &form.environment)?;
    if let Some(interval) = parse_start_interval(&form.start_interval)? {
        dict.insert("StartInterval".into(), Value::Integer(interval.into()));
    }

    Value::Dictionary(dict)
        .to_file_xml(&path)
        .with_context(|| format!("write {}", path.display()))?;

    lint_plist(&path)?;

    let mut steps = Vec::new();
    if form.bootstrap_now {
        steps.push(run_launchctl(&[
            "bootstrap",
            &current_gui_domain()?,
            &path.display().to_string(),
        ]));
    }
    if form.start_now {
        steps.push(run_launchctl(&[
            "kickstart",
            &format!("{}/{}", current_gui_domain()?, form.label.trim()),
        ]));
    }

    Ok(CreateOutcome { path, steps })
}

fn validate(form: &CreateServiceForm) -> Result<()> {
    let label = form.label.trim();
    if label.is_empty() {
        bail!("label is required");
    }
    if !label
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        bail!("label may only contain letters, numbers, '.', '-' and '_'");
    }

    let command = form.command.trim();
    if command.is_empty() {
        bail!("command is required");
    }
    let command_path = PathBuf::from(command);
    if !command_path.is_absolute() {
        bail!("command must be an absolute path");
    }
    if !command_path.exists() {
        bail!("command does not exist: {command}");
    }

    validate_parent("working directory", &form.working_directory, true)?;
    validate_parent("stdout path", &form.stdout_path, false)?;
    validate_parent("stderr path", &form.stderr_path, false)?;
    parse_argument_tail(&form.arguments)?;
    parse_environment(&form.environment)?;
    parse_start_interval(&form.start_interval)?;
    if form.start_now && !form.bootstrap_now {
        bail!("start now requires bootstrap now");
    }

    Ok(())
}

fn validate_parent(label: &str, raw_path: &str, path_is_directory: bool) -> Result<()> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return Ok(());
    }

    let path = PathBuf::from(raw_path);
    if path_is_directory {
        if !path.is_dir() {
            bail!("{label} does not exist: {raw_path}");
        }
        return Ok(());
    }

    let Some(parent) = path.parent() else {
        bail!("{label} has no parent directory: {raw_path}");
    };
    if !parent.is_dir() {
        bail!("{label} parent does not exist: {}", parent.display());
    }
    Ok(())
}

fn program_arguments(form: &CreateServiceForm) -> Result<Vec<Value>> {
    Ok(program_argument_strings(form)?
        .into_iter()
        .map(Value::String)
        .collect())
}

fn program_argument_strings(form: &CreateServiceForm) -> Result<Vec<String>> {
    let mut args = vec![form.command.trim().to_string()];
    args.extend(parse_argument_tail(&form.arguments)?);
    Ok(args)
}

fn insert_environment(dict: &mut Dictionary, raw_environment: &str) -> Result<()> {
    let environment = parse_environment(raw_environment)?;
    if environment.is_empty() {
        return Ok(());
    }

    let mut env_dict = Dictionary::new();
    for (key, value) in environment {
        env_dict.insert(key, Value::String(value));
    }
    dict.insert("EnvironmentVariables".into(), Value::Dictionary(env_dict));
    Ok(())
}

fn parse_environment(raw_environment: &str) -> Result<Vec<(String, String)>> {
    if raw_environment.trim().is_empty() {
        return Ok(Vec::new());
    }

    let assignments = shell_words::split(raw_environment)
        .context("environment must use valid shell-style quoting")?;
    let mut parsed = Vec::new();
    for assignment in assignments {
        let Some((key, value)) = assignment.split_once('=') else {
            bail!("environment entry must be KEY=value: {assignment}");
        };
        if key.is_empty() {
            bail!("environment variable name cannot be empty");
        }
        if !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            bail!("environment variable name may only contain letters, numbers, and '_': {key}");
        }
        parsed.push((key.to_string(), value.to_string()));
    }
    Ok(parsed)
}

fn parse_start_interval(raw_interval: &str) -> Result<Option<u64>> {
    let raw_interval = raw_interval.trim();
    if raw_interval.is_empty() {
        return Ok(None);
    }

    let interval = raw_interval
        .parse::<u64>()
        .with_context(|| format!("start interval must be a positive integer: {raw_interval}"))?;
    if interval == 0 {
        bail!("start interval must be greater than 0");
    }
    Ok(Some(interval))
}

fn parse_argument_tail(raw_arguments: &str) -> Result<Vec<String>> {
    if raw_arguments.trim().is_empty() {
        return Ok(Vec::new());
    }

    shell_words::split(raw_arguments).context("arguments must use valid shell-style quoting")
}

fn insert_optional_string(dict: &mut Dictionary, key: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        dict.insert(key.into(), Value::String(value.to_string()));
    }
}

fn lint_plist(path: &PathBuf) -> Result<()> {
    let output = Command::new("plutil").arg("-lint").arg(path).output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("plutil failed: {}", stderr.trim());
        }
        Err(err) => bail!("plutil failed: {err}"),
    }
}

fn current_gui_domain() -> Result<String> {
    let output = Command::new("id").arg("-u").output().context("run id -u")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("id -u failed: {}", stderr.trim());
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uid.is_empty() {
        bail!("id -u returned an empty uid");
    }
    Ok(format!("gui/{uid}"))
}

fn run_launchctl(args: &[&str]) -> String {
    let output = Command::new("launchctl").args(args).output();
    let command = format!("launchctl {}", args.join(" "));
    match output {
        Ok(output) if output.status.success() => format!("{command}: ok"),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            format!("{command}: failed {message}")
        }
        Err(err) => format!("{command}: failed {err}"),
    }
}

fn empty_marker(value: &str) -> String {
    if value.is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    }
}

fn required_marker(value: &str) -> String {
    if value.is_empty() {
        "(required)".to_string()
    } else {
        value.to_string()
    }
}

fn bool_marker(value: bool) -> String {
    if value {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form_with_arguments(arguments: &str) -> CreateServiceForm {
        let mut form = CreateServiceForm::new();
        form.label = "com.sderosiaux.launchdeck.test".to_string();
        form.command = "/bin/echo".to_string();
        form.arguments = arguments.to_string();
        form
    }

    #[test]
    fn program_arguments_preserve_shell_quoted_values() {
        let form = form_with_arguments(r#"--name "hello world" 'single quoted' plain"#);

        let args = program_argument_strings(&form).expect("arguments parse");

        assert_eq!(
            args,
            vec![
                "/bin/echo",
                "--name",
                "hello world",
                "single quoted",
                "plain"
            ]
        );
    }

    #[test]
    fn invalid_argument_quoting_is_rejected() {
        let form = form_with_arguments(r#""unterminated"#);

        let err = validate(&form).expect_err("unterminated quotes should fail");

        assert!(err.to_string().contains("shell-style quoting"));
    }

    #[test]
    fn environment_preserves_quoted_values() {
        let environment = parse_environment(r#"GREETING="hello world" EMPTY= PLAIN=value"#)
            .expect("environment parses");

        assert_eq!(
            environment,
            vec![
                ("GREETING".to_string(), "hello world".to_string()),
                ("EMPTY".to_string(), "".to_string()),
                ("PLAIN".to_string(), "value".to_string())
            ]
        );
    }

    #[test]
    fn invalid_environment_assignment_is_rejected() {
        let err = parse_environment("NO_EQUALS").expect_err("assignment should fail");

        assert!(err.to_string().contains("KEY=value"));
    }

    #[test]
    fn start_interval_must_be_positive() {
        assert_eq!(parse_start_interval("").unwrap(), None);
        assert_eq!(parse_start_interval("60").unwrap(), Some(60));

        let err = parse_start_interval("0").expect_err("zero interval should fail");
        assert!(err.to_string().contains("greater than 0"));
    }

    #[test]
    fn start_now_enables_bootstrap_now() {
        let mut form = form_with_arguments("");
        form.selected = FIELDS
            .iter()
            .position(|field| *field == CreateField::StartNow)
            .unwrap();

        form.toggle();

        assert!(form.start_now);
        assert!(form.bootstrap_now);
    }

    #[test]
    fn disabling_bootstrap_disables_start_now() {
        let mut form = form_with_arguments("");
        form.bootstrap_now = true;
        form.start_now = true;
        form.selected = FIELDS
            .iter()
            .position(|field| *field == CreateField::BootstrapNow)
            .unwrap();

        form.toggle();

        assert!(!form.bootstrap_now);
        assert!(!form.start_now);
    }
}
