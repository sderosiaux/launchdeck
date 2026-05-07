use super::{App, DETAIL_ITEMS, LogStream, ViewMode};
use crate::actions::ActionKind;
use crate::create;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    if matches!(key.code, KeyCode::Char('?') | KeyCode::F(1)) {
        app.show_help = !app.show_help;
        app.status_line = if app.show_help {
            "help opened".to_string()
        } else {
            "help closed".to_string()
        };
        return false;
    }

    if app.show_help {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Enter => {
                app.show_help = false;
                app.status_line = "help closed".to_string();
            }
            _ => {}
        }
        return false;
    }

    if app.create_form.is_some() {
        handle_create_key(app, key);
        return false;
    }

    if app.pending_action.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => app.confirm_action(),
            KeyCode::Char('n') | KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                app.cancel_action()
            }
            _ => {}
        }
        return false;
    }

    if app.editing_search {
        return handle_search_key(app, key);
    }

    if app.mode == ViewMode::Logs {
        return handle_logs_key(app, key);
    }

    if app.mode == ViewMode::Detail {
        return handle_detail_key(app, key);
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('/') => app.open_search(),
        KeyCode::Char('C') => app.clear_search(),
        KeyCode::Char('F') => app.cycle_status_filter(),
        KeyCode::Char('P') => app.cycle_source_filter(),
        KeyCode::Char('O') => app.cycle_sort(),
        KeyCode::Char('A') => app.toggle_apple(),
        KeyCode::Char('W') => app.toggle_warnings_only(),
        KeyCode::Char('Y') => app.copy_selected_value(),
        KeyCode::Char('L') if app.selected_service().is_some() => app.open_logs(LogStream::Stdout),
        KeyCode::Char('R') => app.plan_action(ActionKind::Restart),
        KeyCode::Char('S') => app.plan_action(ActionKind::Start),
        KeyCode::Char('X') => app.plan_action(ActionKind::Stop),
        KeyCode::Char('T') => app.plan_action(ActionKind::ToggleEnabled),
        KeyCode::Char('U') => app.plan_action(ActionKind::ToggleRunAtLoad),
        KeyCode::Char('E') => app.plan_action(ActionKind::EditPlist),
        KeyCode::Char('D') => app.plan_action(ActionKind::Delete),
        KeyCode::Char('N') => app.open_create_form(),
        KeyCode::F(5) => {
            app.refresh_requested = true;
            app.status_line = "refresh requested".to_string();
        }
        KeyCode::Char('r') if has_command_modifier(key) => {
            app.refresh_requested = true;
            app.status_line = "refresh requested".to_string();
        }
        KeyCode::Down => app.move_down(),
        KeyCode::Up => app.move_up(),
        KeyCode::PageDown => app.page_down(),
        KeyCode::PageUp => app.page_up(),
        KeyCode::Enter if app.selected_service().is_some() => app.open_detail(),
        KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => app.mode = ViewMode::Overview,
        KeyCode::Char(value) if is_quick_search_key(key) => app.start_quick_search(value),
        _ => {}
    }

    false
}

fn handle_detail_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => app.close_detail(),
        KeyCode::Char('j') | KeyCode::Down => app.move_detail_down(),
        KeyCode::Char('k') | KeyCode::Up => app.move_detail_up(),
        KeyCode::PageDown => app.page_detail_down(),
        KeyCode::PageUp => app.page_detail_up(),
        KeyCode::Char('g') => app.detail_selected = 0,
        KeyCode::Char('G') => app.detail_selected = DETAIL_ITEMS.len() - 1,
        KeyCode::Char('c') => app.copy_selected_value(),
        KeyCode::Enter => app.open_selected_detail_item(),
        KeyCode::Char('l') => app.open_logs(LogStream::Stdout),
        KeyCode::Char('s') => app.plan_action(ActionKind::Start),
        KeyCode::Char('x') => app.plan_action(ActionKind::Stop),
        KeyCode::Char('R') => app.plan_action(ActionKind::Restart),
        KeyCode::Char('e') => app.plan_action(ActionKind::ToggleEnabled),
        KeyCode::Char('u') => app.plan_action(ActionKind::ToggleRunAtLoad),
        KeyCode::Char('E') => app.plan_action(ActionKind::EditPlist),
        KeyCode::Char('D') => app.plan_action(ActionKind::Delete),
        _ => {}
    }
    false
}

fn is_quick_search_key(key: KeyEvent) -> bool {
    let KeyCode::Char(value) = key.code else {
        return false;
    };
    !value.is_control() && !has_command_modifier(key)
}

fn has_command_modifier(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT)
}

fn handle_create_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        app.save_create_form();
        return;
    }

    match key.code {
        KeyCode::Esc => app.cancel_create_form(),
        KeyCode::F(5) => app.save_create_form(),
        _ => {
            let Some(form) = app.create_form.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Tab | KeyCode::Down | KeyCode::Right | KeyCode::Enter => form.next(),
                KeyCode::BackTab | KeyCode::Up | KeyCode::Left => form.previous(),
                KeyCode::Backspace => {
                    let before = form.value_for(form.current_field());
                    form.backspace();
                    if before == form.value_for(form.current_field()) {
                        form.previous();
                    }
                }
                KeyCode::Char(' ') => {
                    if matches!(
                        form.current_field(),
                        create::CreateField::RunAtLoad
                            | create::CreateField::KeepAlive
                            | create::CreateField::BootstrapNow
                            | create::CreateField::StartNow
                    ) {
                        form.toggle();
                    } else {
                        form.insert(' ');
                    }
                }
                KeyCode::Char(value) => form.insert(value),
                _ => {}
            }
        }
    }
}

fn handle_logs_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Esc | KeyCode::Backspace => app.mode = ViewMode::Detail,
        KeyCode::Char('j') | KeyCode::Down => app.scroll_logs_down(1),
        KeyCode::Char('k') | KeyCode::Up => app.scroll_logs_up(1),
        KeyCode::PageDown => app.scroll_logs_down(10),
        KeyCode::PageUp => app.scroll_logs_up(10),
        KeyCode::Char('G') => app.log_scroll = 0,
        KeyCode::Char('g') => app.log_scroll = usize::MAX / 2,
        KeyCode::Char('c') => app.copy_selected_value(),
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => app.switch_log_stream(),
        KeyCode::Char('l') => app.switch_log_stream(),
        KeyCode::Char('r') => app.refresh_unified_log(),
        KeyCode::Char('w') => app.cycle_unified_window(),
        _ => {}
    }
    false
}

fn handle_search_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Left => {
            app.editing_search = false;
            app.status_line = "search cancelled".to_string();
        }
        KeyCode::Enter => {
            app.editing_search = false;
            if app.selected_service().is_some() {
                app.open_detail();
            } else {
                app.status_line = format!("{} services match", app.filtered.len());
            }
        }
        KeyCode::Down => app.move_down(),
        KeyCode::Up => app.move_up(),
        KeyCode::PageDown => app.page_down(),
        KeyCode::PageUp => app.page_up(),
        KeyCode::Char('C') => {
            app.editing_search = false;
            app.clear_search();
        }
        KeyCode::Backspace => {
            app.search.pop();
            app.selected = 0;
            app.viewport_start = 0;
            app.apply_filter();
        }
        KeyCode::Char(value) => {
            app.search.push(value);
            app.selected = 0;
            app.viewport_start = 0;
            app.apply_filter();
        }
        _ => {}
    }
    false
}
