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
    Stdout,
    Stderr,
    RunAtLoad,
    KeepAlive,
}

impl CreateField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Label => "Label",
            Self::Command => "Command",
            Self::Arguments => "Arguments",
            Self::WorkingDirectory => "Working directory",
            Self::Stdout => "Stdout path",
            Self::Stderr => "Stderr path",
            Self::RunAtLoad => "Run at load",
            Self::KeepAlive => "Keep alive",
        }
    }

    fn editable(self) -> bool {
        !matches!(self, Self::RunAtLoad | Self::KeepAlive)
    }
}

const FIELDS: [CreateField; 8] = [
    CreateField::Label,
    CreateField::Command,
    CreateField::Arguments,
    CreateField::WorkingDirectory,
    CreateField::Stdout,
    CreateField::Stderr,
    CreateField::RunAtLoad,
    CreateField::KeepAlive,
];

#[derive(Clone, Debug)]
pub struct CreateServiceForm {
    pub label: String,
    pub command: String,
    pub arguments: String,
    pub working_directory: String,
    pub stdout_path: String,
    pub stderr_path: String,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub selected: usize,
}

impl CreateServiceForm {
    pub fn new() -> Self {
        Self {
            label: String::new(),
            command: String::new(),
            arguments: String::new(),
            working_directory: String::new(),
            stdout_path: String::new(),
            stderr_path: String::new(),
            run_at_load: false,
            keep_alive: false,
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
            CreateField::Stdout => empty_marker(&self.stdout_path),
            CreateField::Stderr => empty_marker(&self.stderr_path),
            CreateField::RunAtLoad => bool_marker(self.run_at_load),
            CreateField::KeepAlive => bool_marker(self.keep_alive),
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
            CreateField::Stdout => &mut self.stdout_path,
            CreateField::Stderr => &mut self.stderr_path,
            CreateField::RunAtLoad | CreateField::KeepAlive => unreachable!(),
        }
    }
}

pub fn write_user_agent(form: &CreateServiceForm) -> Result<PathBuf> {
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

    Value::Dictionary(dict)
        .to_file_xml(&path)
        .with_context(|| format!("write {}", path.display()))?;

    lint_plist(&path)?;
    Ok(path)
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
}
