mod actions;
mod app;
mod create;
mod discovery;
mod model;
mod oslog;
mod ui;

use anyhow::Result;
use model::{Inventory, Service};
use std::env;
use std::io::{self, Write};

fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }

    if is_list_command(&args) {
        let show_apple = args.iter().any(|arg| arg == "--all" || arg == "-a");
        if let Err(err) = print_inventory(discovery::load_inventory(), show_apple)
            && err.kind() != io::ErrorKind::BrokenPipe
        {
            return Err(err.into());
        }
        return Ok(());
    }

    app::run()
}

fn is_list_command(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--list") || args.first().is_some_and(|arg| arg == "list")
}

fn print_help() {
    println!("Launchdeck");
    println!();
    println!("Usage:");
    println!("  launchdeck              open the TUI");
    println!("  launchdeck list         print services visible by default in the TUI");
    println!("  launchdeck list --all   print all discovered services, including Apple/system");
    println!("  launchdeck --list       alias for launchdeck list");
    println!("  launchdeck --help       show this help");
}

fn print_inventory(inventory: Inventory, show_apple: bool) -> io::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(
        stdout,
        "{:<10} {:<7} {:<13} {:<8} {:<6} {:<24} NAME",
        "STATUS", "SOURCE", "SCOPE", "PID", "LOAD", "SCHEDULE"
    )?;
    for service in inventory.services {
        if !show_apple && is_apple_service(&service) {
            continue;
        }
        writeln!(
            stdout,
            "{:<10} {:<7} {:<13} {:<8} {:<6} {:<24} {}",
            service.status,
            service.source,
            service.scope.label(),
            service
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run_at_load_label(service.config.run_at_load),
            service.config.schedule_summary(),
            service.display_name
        )?;
    }

    if !inventory.warnings.is_empty() {
        eprintln!();
        eprintln!("Warnings:");
        for warning in inventory.warnings {
            eprintln!("- {warning}");
        }
    }

    Ok(())
}

fn is_apple_service(service: &Service) -> bool {
    service.label.starts_with("com.apple.")
        || service
            .plist_path
            .as_ref()
            .is_some_and(|path| path.starts_with("/System/Library"))
}

fn run_at_load_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
}
