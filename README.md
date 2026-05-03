# Launchdeck

Launchdeck is a keyboard-first macOS TUI for managing `launchd` jobs and Homebrew services from one place.

The working idea is simple: most developer-facing macOS background processes are either raw `launchd` plists or Homebrew services backed by `launchd` plists. The tool should show both through one service overview, make common lifecycle actions safe, and keep the escape hatch to inspect the underlying plist and command output.

## Initial Scope

- Service overview across user agents, global agents, system daemons, and Homebrew services.
- Real-time-ish status refresh using `launchctl`, `brew services --json`, and filesystem watches.
- Start, stop, restart, enable, disable, bootstrap, and bootout actions with safety checks.
- Structured service creation for common agent and daemon cases.
- Integrated logs from service stdout/stderr files and macOS unified logging.
- Smart search and filters by source, domain, status, ownership, safety level, and tags.

See [docs/SPEC.md](docs/SPEC.md) for the working product and technical spec.

## Run

```sh
cargo run
```

Useful non-interactive check:

```sh
cargo run -- --list
```

Optional fake user agent for local testing:

```sh
cp fixtures/com.sderosiaux.launchdeck.fake.plist ~/Library/LaunchAgents/
cargo run -- --list | rg launchdeck.fake
```

The fixture is not bootstrapped by default. It is just a plist file for discovery, detail view, log path, and action-planning tests.

## Current MVP

The first implementation discovers plist-backed `launchd` jobs, merges Homebrew services from `brew services list --json`, parses runtime state from `launchctl`, and renders a TUI overview with search, source filters, status filters, detail view, and log previews from configured stdout/stderr paths. Automatic refresh runs in the background so scrolling stays responsive.

Lifecycle actions are guarded by a confirmation modal that shows the exact command before execution. Homebrew services use `brew services`; user-owned launchd jobs use `launchctl`. Read-only system services, admin-required services, and vendor/runtime services are blocked until the privilege/safety model is stronger.
