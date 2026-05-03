use crate::app::{App, ViewMode};
use crate::model::{Service, ServiceStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(area);

    draw_header(frame, app, chunks[0]);
    draw_overview(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

    match app.mode {
        ViewMode::Overview => {}
        ViewMode::Detail => draw_detail(frame, app),
        ViewMode::Logs => draw_logs(frame, app),
    }

    if app.pending_action.is_some() {
        draw_action(frame, app);
    }
}

fn draw_header(frame: &mut Frame<'_>, app: &App, area: ratatui::layout::Rect) {
    let (total, running, failed, warnings) = app.counts();
    let search = if app.search.is_empty() {
        "-".to_string()
    } else {
        app.search.clone()
    };
    let title = Line::from(vec![
        Span::styled("Launchdeck", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "  svc {total} | run {running} | fail {failed} | warn {warnings} | src {} | status {} | sort {} | apple {} | warn {} | / {search}",
            app.source_filter.label(),
            app.status_filter.label(),
            app.sort_mode.label(),
            if app.show_apple { "on" } else { "off" },
            if app.warnings_only { "on" } else { "off" },
        )),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_overview(frame: &mut Frame<'_>, app: &mut App, area: ratatui::layout::Rect) {
    let visible_rows = usize::from(area.height.saturating_sub(3)).max(1);
    sync_viewport(app, visible_rows);
    let start = app.viewport_start;
    let end = (start + visible_rows).min(app.filtered.len());

    let header = Row::new([
        Cell::from("status"),
        Cell::from("name"),
        Cell::from("source"),
        Cell::from("scope"),
        Cell::from("pid"),
        Cell::from("exit"),
        Cell::from("health"),
        Cell::from("path"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let rows =
        app.filtered[start..end]
            .iter()
            .enumerate()
            .filter_map(|(row_index, service_index)| {
                let absolute_row = start + row_index;
                let service = app.services.get(*service_index)?;
                let selected = absolute_row == app.selected;
                let marker = if selected { "> " } else { "  " };
                let path = service
                    .plist_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .or_else(|| service.brew_formula.clone())
                    .unwrap_or_else(|| "-".to_string());
                let style = if selected {
                    Style::default()
                        .add_modifier(Modifier::REVERSED)
                        .add_modifier(Modifier::BOLD)
                } else {
                    status_style(&service.status)
                };
                Some(
                    Row::new([
                        Cell::from(format!("{marker}{}", service.status)),
                        Cell::from(service.display_name.clone()),
                        Cell::from(service.source.to_string()),
                        Cell::from(service.scope.label()),
                        Cell::from(
                            service
                                .pid
                                .map(|pid| pid.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                        ),
                        Cell::from(
                            service
                                .exit_code
                                .map(|code| code.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                        ),
                        Cell::from(service.health.len().to_string()),
                        Cell::from(path),
                    ])
                    .style(style),
                )
            });

    let table = Table::new(
        rows,
        [
            Constraint::Length(13),
            Constraint::Percentage(18),
            Constraint::Length(8),
            Constraint::Length(13),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Percentage(45),
        ],
    )
    .header(header)
    .block(Block::default().title("Services").borders(Borders::ALL))
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_widget(table, area);
}

fn sync_viewport(app: &mut App, visible_rows: usize) {
    if app.filtered.is_empty() {
        app.viewport_start = 0;
        return;
    }

    let max_start = app.filtered.len().saturating_sub(visible_rows);
    app.viewport_start = app.viewport_start.min(max_start);

    if app.selected < app.viewport_start {
        app.viewport_start = app.selected;
    } else if app.selected >= app.viewport_start + visible_rows {
        app.viewport_start = app.selected + 1 - visible_rows;
    }
}

fn draw_footer(frame: &mut Frame<'_>, app: &App, area: ratatui::layout::Rect) {
    let keys = if app.editing_search {
        "enter apply | esc cancel | backspace edit"
    } else if app.pending_action.is_some() {
        "y confirm | n cancel | esc cancel"
    } else {
        "q quit | / find | c clear | f source | F status | o sort | a apple | w warn | PgUp/PgDn"
    };
    let text = vec![
        Line::from(keys),
        Line::from(format!("status: {}", app.status_line)),
    ];
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_detail(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 82, 78);
    frame.render_widget(Clear, area);

    let Some(service) = selected_service(app) else {
        return;
    };

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            service.display_name.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(service.label.clone(), Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::from(""));
    lines.push(kv_line(
        "status",
        service.status.to_string(),
        status_style(&service.status),
    ));
    lines.push(kv_line(
        "source",
        service.source.to_string(),
        Style::default().fg(Color::Blue),
    ));
    lines.push(kv_line(
        "domain",
        service.domain.clone(),
        Style::default().fg(Color::Magenta),
    ));
    lines.push(kv_line(
        "scope",
        service.scope.label().to_string(),
        Style::default().fg(Color::Magenta),
    ));
    lines.push(kv_line(
        "safety",
        service.safety_level.to_string(),
        Style::default().fg(Color::Yellow),
    ));
    lines.push(kv_line(
        "plist",
        service
            .plist_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string()),
        Style::default().fg(Color::Green),
    ));
    lines.push(kv_line(
        "brew formula",
        service.brew_formula.as_deref().unwrap_or("-").to_string(),
        Style::default().fg(Color::Blue),
    ));
    lines.push(kv_line(
        "brew status",
        service.brew_status.as_deref().unwrap_or("-").to_string(),
        Style::default().fg(Color::Blue),
    ));
    lines.push(Line::from(""));
    lines.push(kv_line(
        "command",
        service.config.command_preview(),
        Style::default().fg(Color::Green),
    ));
    lines.push(kv_line(
        "working directory",
        service
            .config
            .working_directory
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        Style::default().fg(Color::Green),
    ));
    lines.push(kv_line(
        "stdout",
        service
            .config
            .stdout_path
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        Style::default().fg(Color::Green),
    ));
    lines.push(kv_line(
        "stderr",
        service
            .config
            .stderr_path
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        Style::default().fg(Color::Green),
    ));
    lines.push(kv_line(
        "RunAtLoad",
        service
            .config
            .run_at_load
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        Style::default().fg(Color::Yellow),
    ));
    lines.push(kv_line(
        "KeepAlive",
        service
            .config
            .keep_alive
            .as_deref()
            .unwrap_or("-")
            .to_string(),
        Style::default().fg(Color::Yellow),
    ));
    lines.push(kv_line(
        "StartInterval",
        service
            .config
            .start_interval
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        Style::default().fg(Color::Yellow),
    ));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "health",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    if service.health.is_empty() {
        lines.push(Line::from(Span::styled(
            "  clean",
            Style::default().fg(Color::Green),
        )));
    } else {
        for item in &service.health {
            lines.push(Line::from(vec![
                Span::styled("  - ", Style::default().fg(Color::Red)),
                Span::styled(item.clone(), Style::default().fg(Color::Red)),
            ]));
        }
    }

    let block = Block::default()
        .title("Detail - esc closes")
        .borders(Borders::ALL);
    let inner = block.inner(area).inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn kv_line<'a>(key: &'static str, value: String, value_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{key:<18}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(value, value_style),
    ])
}

fn draw_action(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 74, 34);
    frame.render_widget(Clear, area);

    let Some(plan) = &app.pending_action else {
        return;
    };

    let title = if plan.is_blocked() {
        "Action Blocked"
    } else {
        "Confirm Action"
    };
    let mut lines = Vec::new();
    lines.push(Line::from(format!("action: {}", plan.kind.label())));
    lines.push(Line::from(format!("service: {}", plan.service_name)));
    lines.push(Line::from(format!("command: {}", plan.command_display())));
    lines.push(Line::from(""));
    lines.push(Line::from(format!("note: {}", plan.warning)));
    if let Some(reason) = &plan.blocked_reason {
        lines.push(Line::from(format!("blocked: {reason}")));
        lines.push(Line::from(""));
        lines.push(Line::from("press n or esc to close"));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from("press y to run, n or esc to cancel"));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_logs(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 82, 62);
    frame.render_widget(Clear, area);

    let Some(service) = selected_service(app) else {
        return;
    };

    let mut items = Vec::new();
    items.push(ListItem::new(format!("service: {}", service.display_name)));
    items.push(ListItem::new(""));
    add_log_preview(&mut items, "stdout", service.config.stdout_path.as_deref());
    items.push(ListItem::new(""));
    add_log_preview(&mut items, "stderr", service.config.stderr_path.as_deref());
    items.push(ListItem::new(""));
    items.push(ListItem::new("unified log command:"));
    items.push(ListItem::new(format!(
        "log stream --predicate 'process == \"{}\"' --style compact",
        service.display_name
    )));

    let list = List::new(items).block(
        Block::default()
            .title("Logs - esc closes")
            .borders(Borders::ALL),
    );
    frame.render_widget(list, area);
}

fn add_log_preview(items: &mut Vec<ListItem<'_>>, label: &str, path: Option<&str>) {
    let Some(path) = path else {
        items.push(ListItem::new(format!("{label}: -")));
        return;
    };

    items.push(ListItem::new(format!("{label}: {path}")));
    match tail_file(path, 16) {
        Ok(lines) if lines.is_empty() => items.push(ListItem::new("  empty")),
        Ok(lines) => {
            for line in lines {
                items.push(ListItem::new(format!("  {line}")));
            }
        }
        Err(err) => items.push(ListItem::new(format!("  cannot read: {err}"))),
    }
}

fn tail_file(path: &str, max_lines: usize) -> std::io::Result<Vec<String>> {
    let mut file = File::open(Path::new(path))?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(32 * 1024);
    file.seek(SeekFrom::Start(start))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content
        .lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(str::to_string)
        .collect())
}

fn selected_service(app: &App) -> Option<&Service> {
    app.filtered
        .get(app.selected)
        .and_then(|index| app.services.get(*index))
}

fn status_style(status: &ServiceStatus) -> Style {
    match status {
        ServiceStatus::Running => Style::default().fg(Color::Green),
        ServiceStatus::Failed => Style::default().fg(Color::Red),
        ServiceStatus::Disabled => Style::default().fg(Color::Yellow),
        ServiceStatus::Unloaded => Style::default().fg(Color::DarkGray),
        ServiceStatus::Stopped | ServiceStatus::Unknown => Style::default(),
    }
}

fn centered_rect(
    area: ratatui::layout::Rect,
    percent_x: u16,
    percent_y: u16,
) -> ratatui::layout::Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1]);
    horizontal[1]
}
