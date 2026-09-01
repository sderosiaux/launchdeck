use crate::app::{App, DetailItem, ViewMode, detail_item_value};
use crate::create::{CreateField, CreateServiceForm};
use crate::model::{Service, ServiceStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const TAIL_WINDOW_BYTES: u64 = 32 * 1024;

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
    if app.create_form.is_some() {
        draw_create_form(frame, app);
    }
    if app.show_help {
        draw_help(frame);
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
        Cell::from("load"),
        Cell::from("schedule"),
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
                        Cell::from(run_at_load_label(service.config.run_at_load)),
                        Cell::from(service.config.schedule_summary()),
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
            Constraint::Length(19),
            Constraint::Percentage(43),
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
        "type filter | arrows/PgUp/PgDn move | enter detail | C clear | esc done"
    } else if app.create_form.is_some() {
        "tab/down/right next | shift-tab/up/left prev | space toggle | ctrl+s/F5 save | esc cancel"
    } else if app.pending_action.is_some() {
        "y/enter confirm | n/esc/left/backspace cancel"
    } else if app.mode == ViewMode::Logs {
        "k/up older | j/down newer | PgUp/PgDn | g/G | tab stream | c copy path | esc back"
    } else if app.mode == ViewMode::Detail {
        "up/down detail | enter acts on field | c copy | s/x/R/e/u | E edit | D delete | esc"
    } else {
        "type search | arrows move | enter detail | Y copy | S/X/R/T/U | E edit | D delete"
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

fn draw_detail(frame: &mut Frame<'_>, app: &mut App) {
    let area = centered_rect(frame.area(), 82, 78);
    frame.render_widget(Clear, area);

    let Some(service) = selected_service(app).cloned() else {
        return;
    };

    let block = Block::default()
        .title("Detail - esc closes")
        .borders(Borders::ALL);
    let inner = block.inner(area).inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    frame.render_widget(block, area);

    let header = vec![
        Line::from(vec![
            Span::styled(
                service.display_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(service.label.clone(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];

    let visible_rows = usize::from(inner.height.saturating_sub(header.len() as u16)).max(1);
    sync_detail_viewport(app, visible_rows);

    let mut body = Vec::new();
    for (index, item) in App::detail_items().iter().enumerate() {
        body.push(detail_item_line(
            &service,
            *item,
            index == app.detail_selected,
        ));
    }

    let start = app.detail_scroll.min(body.len().saturating_sub(1));
    let end = (start + visible_rows).min(body.len());
    let mut lines = header;
    lines.extend(body[start..end].iter().cloned());

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn sync_detail_viewport(app: &mut App, visible_rows: usize) {
    let item_count = App::detail_items().len();
    let max_start = item_count.saturating_sub(visible_rows);
    app.detail_scroll = app.detail_scroll.min(max_start);

    if app.detail_selected < app.detail_scroll {
        app.detail_scroll = app.detail_selected;
    } else if app.detail_selected >= app.detail_scroll + visible_rows {
        app.detail_scroll = app.detail_selected + 1 - visible_rows;
    }
}

fn detail_item_line<'a>(service: &Service, item: DetailItem, selected: bool) -> Line<'a> {
    // Same source as the copy action, so what you read is what you copy.
    let value = detail_item_value(service, item);

    let value_style = match item {
        DetailItem::Status => status_style(&service.status),
        DetailItem::Source | DetailItem::BrewFormula | DetailItem::BrewStatus => {
            Style::default().fg(Color::Blue)
        }
        DetailItem::Domain | DetailItem::Scope | DetailItem::Origin => {
            Style::default().fg(Color::Magenta)
        }
        DetailItem::Elevation => {
            if service.elevation == crate::model::ElevationNeeds::none() {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Yellow)
            }
        }
        DetailItem::Safety
        | DetailItem::RunAtLoad
        | DetailItem::KeepAlive
        | DetailItem::StartInterval
        | DetailItem::Schedule => Style::default().fg(Color::Yellow),
        DetailItem::Plist
        | DetailItem::Command
        | DetailItem::WorkingDirectory
        | DetailItem::Stdout
        | DetailItem::Stderr => Style::default().fg(Color::Green),
        DetailItem::Health => {
            if service.health.is_empty() {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            }
        }
    };

    selectable_kv_line(item.label(), value, value_style, selected)
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

fn selectable_kv_line<'a>(
    key: &'static str,
    value: String,
    value_style: Style,
    selected: bool,
) -> Line<'a> {
    let marker = if selected { "> " } else { "  " };
    let key_style = if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };

    Line::from(vec![
        Span::styled(format!("{marker}{key:<18}"), key_style),
        Span::styled(value, value_style),
    ])
}

fn draw_action(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 78, 42);
    frame.render_widget(Clear, area);

    let Some(plan) = &app.pending_action else {
        return;
    };

    let title = if plan.is_blocked() {
        "Action Blocked"
    } else {
        "Confirm Action"
    };
    let title_style = if plan.is_blocked() {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(title, title_style),
            Span::raw("  "),
            Span::styled(plan.kind.label(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        kv_line(
            "action",
            plan.kind.label().to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        kv_line(
            "service",
            plan.service_name.clone(),
            Style::default().fg(Color::Blue),
        ),
        kv_line(
            "command",
            plan.command_display(),
            Style::default().fg(Color::Green),
        ),
        Line::from(""),
        kv_line(
            "note",
            plan.warning.clone(),
            Style::default().fg(Color::Yellow),
        ),
    ];
    if let Some(reason) = &plan.blocked_reason {
        lines.push(Line::from(""));
        lines.push(kv_line(
            "blocked",
            reason.clone(),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("n", Style::default().fg(Color::Cyan)),
            Span::raw(" / "),
            Span::styled("esc", Style::default().fg(Color::Cyan)),
            Span::raw(" close"),
        ]));
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" / "),
            Span::styled("enter", Style::default().fg(Color::Green)),
            Span::raw(" run   "),
            Span::styled("n", Style::default().fg(Color::Cyan)),
            Span::raw(" / "),
            Span::styled("esc", Style::default().fg(Color::Cyan)),
            Span::raw(" cancel"),
        ]));
    }

    let block = Block::default()
        .title(title)
        .title_style(title_style)
        .borders(Borders::ALL);
    let inner = block.inner(area).inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(block, area);
    frame.render_widget(paragraph, inner);
}

fn draw_create_form(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 84, 72);
    frame.render_widget(Clear, area);

    let Some(form) = &app.create_form else {
        return;
    };

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "New user LaunchAgent",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "~/Library/LaunchAgents",
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(""));

    for field in CreateServiceForm::fields() {
        lines.push(create_field_line(form, *field));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "Creation is user-only. ",
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("Optional bootstrap/start runs launchctl in the current gui domain."),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Arguments ", Style::default().fg(Color::DarkGray)),
        Span::raw(r#"accept shell-style quotes, for example --name "hello world"."#),
    ]));

    let block = Block::default()
        .title("Create service")
        .borders(Borders::ALL);
    let inner = block.inner(area).inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_help(frame: &mut Frame<'_>) {
    let area = centered_rect(frame.area(), 78, 74);
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        help_line("Global", "q quit | ?/F1 help | esc/backspace/left back"),
        help_line(
            "Overview",
            "type to search | arrows move | PgUp/PgDn page | enter detail",
        ),
        help_line(
            "Filters",
            "/ search | C clear | P source | F status | O sort | A apple | W warnings",
        ),
        help_line(
            "Copy",
            "Y copies the selected service; c copies detail values and log paths",
        ),
        help_line(
            "Actions",
            "S start | X stop | R restart/load | T enable/disable | U RunAtLoad",
        ),
        help_line("Plist", "E edit plist | D delete plist after confirmation"),
        help_line(
            "Detail",
            "j/k or arrows move inside | enter acts on status, plist, RunAtLoad, logs",
        ),
        help_line(
            "Logs",
            "j/down newer | k/up older | PgUp/PgDn | g top | G bottom | tab cycles stdout/stderr/os_log",
        ),
        help_line(
            "os_log",
            "r refresh from /usr/bin/log | w cycles 5m/15m/1h window",
        ),
        help_line(
            "Create",
            "n new | tab/down/right next | shift-tab/up/left previous",
        ),
        help_line(
            "Create save",
            "space toggles booleans | ctrl+s/F5 save | esc cancel",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Help closes with esc, backspace, left, enter, ? or F1.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default().title("Help").borders(Borders::ALL);
    let inner = block.inner(area).inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn help_line<'a>(label: &'static str, value: &'static str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{label:<13}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(value),
    ])
}

fn create_field_line<'a>(form: &CreateServiceForm, field: CreateField) -> Line<'a> {
    let selected = form.current_field() == field;
    let marker = if selected { "> " } else { "  " };
    let label_style = if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };
    let value_style = match field {
        CreateField::RunAtLoad
        | CreateField::KeepAlive
        | CreateField::BootstrapNow
        | CreateField::StartNow => Style::default().fg(Color::Yellow),
        CreateField::Command
        | CreateField::WorkingDirectory
        | CreateField::Environment
        | CreateField::StartInterval
        | CreateField::Stdout
        | CreateField::Stderr => Style::default().fg(Color::Green),
        CreateField::Label | CreateField::Arguments => Style::default().fg(Color::Cyan),
    };
    Line::from(vec![
        Span::styled(format!("{marker}{:<18}", field.label()), label_style),
        Span::styled(form.value_for(field), value_style),
    ])
}

fn draw_logs(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 86, 72);
    frame.render_widget(Clear, area);

    let Some(service) = selected_service(app) else {
        return;
    };

    let block = Block::default()
        .title("Logs - esc closes")
        .borders(Borders::ALL);
    let inner = block.inner(area).inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    frame.render_widget(block, area);

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                service.display_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            stream_tab("stdout", app.log_stream == crate::app::LogStream::Stdout),
            Span::raw(" "),
            stream_tab("stderr", app.log_stream == crate::app::LogStream::Stderr),
            Span::raw(" "),
            stream_tab("os_log", app.log_stream == crate::app::LogStream::Unified),
        ]),
        Line::from(""),
    ];

    let visible_log_rows = usize::from(inner.height.saturating_sub(6)).max(1);
    match app.log_stream {
        crate::app::LogStream::Unified => render_unified(&mut lines, app, visible_log_rows),
        stream => {
            let path = match stream {
                crate::app::LogStream::Stdout => service.config.stdout_path.as_deref(),
                crate::app::LogStream::Stderr => service.config.stderr_path.as_deref(),
                crate::app::LogStream::Unified => unreachable!(),
            };
            render_file_stream(&mut lines, app, path, visible_log_rows);
        }
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_file_stream<'a>(
    lines: &mut Vec<Line<'a>>,
    app: &App,
    path: Option<&str>,
    visible_log_rows: usize,
) {
    lines.push(kv_line(
        "path",
        path.unwrap_or("-").to_string(),
        Style::default().fg(Color::Green),
    ));
    match path {
        Some(path) => match tail_file(path, 500) {
            Ok(file_lines) => {
                let max_scroll = file_lines.len().saturating_sub(visible_log_rows);
                let scroll = app.log_scroll.min(max_scroll);
                let start = file_lines.len().saturating_sub(visible_log_rows + scroll);
                let end = (start + visible_log_rows).min(file_lines.len());
                lines.push(kv_line(
                    "view",
                    format!(
                        "{}-{} of {}",
                        start.saturating_add(1).min(file_lines.len()),
                        end,
                        file_lines.len()
                    ),
                    Style::default().fg(Color::Yellow),
                ));
                lines.push(Line::from(""));
                if file_lines.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "empty log file",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for line in &file_lines[start..end] {
                        lines.push(log_line(line));
                    }
                }
            }
            Err(err) => {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("cannot read: ", Style::default().fg(Color::Red)),
                    Span::styled(err.to_string(), Style::default().fg(Color::Red)),
                ]));
            }
        },
        None => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "no log path configured for this stream",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
}

fn render_unified<'a>(lines: &mut Vec<Line<'a>>, app: &App, visible_log_rows: usize) {
    lines.push(kv_line(
        "window",
        format!("{} (w cycles, r refresh)", app.unified_window.label()),
        Style::default().fg(Color::Green),
    ));

    let Some(cache) = app.unified_cache.as_ref() else {
        lines.push(Line::from(""));
        let msg = if app.unified_in_flight {
            "fetching from /usr/bin/log..."
        } else {
            "press r to fetch os_log"
        };
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::DarkGray),
        )));
        return;
    };

    lines.push(kv_line(
        "predicate",
        cache.predicate.clone(),
        Style::default().fg(Color::DarkGray),
    ));

    if let Some(err) = &cache.error {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("log show error: ", Style::default().fg(Color::Red)),
            Span::styled(err.clone(), Style::default().fg(Color::Red)),
        ]));
        return;
    }

    let total = cache.lines.len();
    let max_scroll = total.saturating_sub(visible_log_rows);
    let scroll = app.log_scroll.min(max_scroll);
    let start = total.saturating_sub(visible_log_rows + scroll);
    let end = (start + visible_log_rows).min(total);
    lines.push(kv_line(
        "view",
        format!(
            "{}-{} of {} ({}ms)",
            start.saturating_add(1).min(total),
            end,
            total,
            cache.elapsed_ms
        ),
        Style::default().fg(Color::Yellow),
    ));
    lines.push(Line::from(""));
    if cache.lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no events in window",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for line in &cache.lines[start..end] {
            lines.push(log_line(line));
        }
    }
}

fn stream_tab<'a>(label: &'static str, selected: bool) -> Span<'a> {
    if selected {
        Span::styled(
            format!("[{label}]"),
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {label} "), Style::default().fg(Color::DarkGray))
    }
}

fn log_line<'a>(line: &str) -> Line<'a> {
    let lower = line.to_lowercase();
    let style = if lower.contains("error") || lower.contains("failed") || lower.contains("panic") {
        Style::default().fg(Color::Red)
    } else if lower.contains("warn") {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Line::from(Span::styled(line.to_string(), style))
}

fn tail_file(path: &str, max_lines: usize) -> std::io::Result<Vec<String>> {
    let mut file = File::open(Path::new(path))?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TAIL_WINDOW_BYTES);
    file.seek(SeekFrom::Start(start))?;
    // Read raw bytes: the seek offset can land mid-codepoint, and service logs are
    // not guaranteed to be UTF-8 at all. Lossy decoding keeps the view usable.
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let content = String::from_utf8_lossy(&buffer);
    // Drop the first line: seeking to a byte offset almost always cuts one in half.
    let content = if start > 0 {
        content.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        content.as_ref()
    };
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
        ServiceStatus::Scheduled => Style::default().fg(Color::Cyan),
        ServiceStatus::Failed => Style::default().fg(Color::Red),
        ServiceStatus::Disabled => Style::default().fg(Color::Yellow),
        ServiceStatus::Unloaded => Style::default().fg(Color::DarkGray),
        ServiceStatus::Stopped | ServiceStatus::Unknown => Style::default(),
    }
}

fn run_at_load_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
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
