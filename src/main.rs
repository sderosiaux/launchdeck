mod actions;
mod app;
mod create;
mod discovery;
mod model;
mod ui;

use anyhow::Result;
use model::Inventory;
use std::env;
use std::io::{self, Write};

fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--list") {
        if let Err(err) = print_inventory(discovery::load_inventory())
            && err.kind() != io::ErrorKind::BrokenPipe
        {
            return Err(err.into());
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }

    app::run()
}

fn print_help() {
    println!("Launchdeck");
    println!();
    println!("Usage:");
    println!("  launchdeck          open the TUI");
    println!("  launchdeck --list   print discovered services");
    println!("  launchdeck --help   show this help");
}

fn print_inventory(inventory: Inventory) -> io::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(
        stdout,
        "{:<10} {:<7} {:<13} {:<8} {:<6} {:<8} {:<24} NAME",
        "STATUS", "SOURCE", "SCOPE", "PID", "EXIT", "HEALTH", "SCHEDULE"
    )?;
    for service in inventory.services {
        writeln!(
            stdout,
            "{:<10} {:<7} {:<13} {:<8} {:<6} {:<8} {:<24} {}",
            service.status,
            service.source,
            service.scope.label(),
            service
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            service
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "-".to_string()),
            service.health.len(),
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
