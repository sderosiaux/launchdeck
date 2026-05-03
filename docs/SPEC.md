# Launchdeck Spec

## Summary

Launchdeck is a terminal UI for macOS services. It manages raw `launchd` jobs and Homebrew services together because they usually meet at the same layer: plist files loaded into `launchd`.

The first version should feel like `systemd-manager-tui`, but native to macOS:

- one overview of every relevant service
- fast search and filtering
- lifecycle controls with explicit confirmation
- plist-aware editing and creation
- logs without leaving the TUI
- safety rules that make system modifications hard to do by accident

The product should start read-heavy and become write-capable only where the behavior is well understood.

## Goals

- Give developers a complete view of local background services without remembering which tool owns each service.
- Treat Homebrew services as first-class services, not as a separate afterthought.
- Make `launchd` visible: domain, label, plist path, PID, last exit code, disabled state, and launch conditions.
- Make common fixes obvious: missing binary, bad permissions, disabled override, stale plist, unloaded service, failing exit code.
- Preserve escape hatches: show the exact command before running it and keep raw plist inspection available.

## Non-Goals

- No edits to `/System/Library/*`.
- No private macOS APIs in the first implementation.
- No background privileged helper in the first implementation.
- No attempt to replace LaunchControl or Lingon as a full GUI plist editor.
- No automatic repair of vendor-owned plists unless the user explicitly asks for that action.

## Users

- Developers running Redis, Postgres, Kafka, Ollama, Cloudflared, Prometheus, or similar tools through Homebrew.
- macOS power users with custom `~/Library/LaunchAgents` jobs.
- Sysadmins who need a fast service/debugging view over `/Library/LaunchAgents` and `/Library/LaunchDaemons`.

## Service Sources

Launchdeck discovers services from:

- `~/Library/LaunchAgents`
- `/Library/LaunchAgents`
- `/Library/LaunchDaemons`
- `/System/Library/LaunchAgents` read-only
- `/System/Library/LaunchDaemons` read-only
- `brew services list --json`
- `brew services info --all --json`
- loaded `launchd` domains from `launchctl print`
- disabled service overrides from `launchctl print-disabled`

Homebrew services are merged with matching plist-backed launchd jobs by plist path, label, and formula name.

## Unified Service Model

Each row in the UI maps to a `Service`:

```text
id: stable internal id
label: launchd label, for example homebrew.mxcl.redis
display_name: user-facing name, for example redis
source: launchd | homebrew | both
domain: gui/<uid> | user/<uid> | system
scope: user_agent | global_agent | system_daemon | system_readonly
plist_path: path to source plist when known
program: resolved executable or app path
arguments: ProgramArguments or Program
working_directory: optional path
pid: current process id when running
status: running | stopped | failed | unloaded | disabled | unknown
exit_code: last exit code when available
enabled: true | false | unknown
loaded: true | false | unknown
brew_formula: optional formula name
brew_status: started | stopped | none | error | unknown
safety_level: user_writable | admin_required | readonly_system | protected_vendor
tags: user-defined strings
health: list of diagnostics
last_refreshed_at: timestamp
```

Status should be derived, not copied blindly from one command:

- `brew services --json` is authoritative for Homebrew formula status.
- `launchctl print <domain>/<label>` is authoritative for loaded `launchd` state.
- plist existence and parse results are authoritative for service definition health.
- `launchctl print-disabled <domain>` is authoritative for disabled state.

When these disagree, the UI should show the conflict instead of hiding it.

## Primary Screens

### Service Overview

The overview is the default screen.

Columns:

- status glyph
- name
- source
- scope
- PID
- exit code
- enabled/disabled
- health count
- plist path or formula

Expected interactions:

- `j/k` or arrows move selection
- `/` search
- `f` opens filter panel
- `r` refreshes now
- `enter` opens detail
- `s` starts selected service
- `x` stops selected service
- `R` restarts selected service
- `e` toggles enabled state
- `l` opens logs
- `n` creates a new service
- `?` shows keymap

Refresh behavior:

- default polling interval: 3 seconds
- manual refresh always available
- filesystem watch for plist directories when supported
- action results trigger immediate refresh

### Detail View

The detail view explains one service clearly.

Sections:

- identity: label, source, domain, scope, owner
- runtime: status, PID, exit code, reason/blame when available
- launch config: program, arguments, environment, schedule, KeepAlive, RunAtLoad
- files: plist, stdout, stderr, working directory
- Homebrew: formula, service file, brew status, brew action commands
- health checks: warnings and suggested commands
- raw output: `launchctl print` and plist view

### Service Creation

Creation should use a form builder, not a blank plist.

Templates:

- user login agent
- user keepalive process
- scheduled command
- path-watched command
- system daemon
- Homebrew service from existing formula file

Required fields:

- label
- command or executable
- arguments
- scope
- run at load
- keep alive

Optional fields:

- working directory
- environment variables
- stdout path
- stderr path
- schedule
- start interval
- start calendar interval
- username for system daemon

Validation before write:

- label is unique in target scope
- target directory exists
- plist is valid XML
- command exists or is clearly marked unresolved
- stdout/stderr parent directories exist
- daemon ownership rules are satisfied
- system daemon does not point to a GUI app
- system-owned paths require admin confirmation

Creation output:

- write plist
- run `plutil -lint`
- optionally bootstrap service
- optionally start service

### Service Management

Actions must show the exact command before execution.

Launchd mappings:

```text
start:    launchctl kickstart <domain>/<label>
stop:     launchctl kill TERM <domain>/<label>
restart:  launchctl kickstart -k <domain>/<label>
enable:   launchctl enable <domain>/<label>
disable:  launchctl disable <domain>/<label>
load:     launchctl bootstrap <domain> <plist_path>
unload:   launchctl bootout <domain>/<label> or launchctl bootout <domain> <plist_path>
inspect:  launchctl print <domain>/<label>
why:      launchctl blame <domain>/<label>
```

Homebrew mappings:

```text
start:    brew services start <formula>
stop:     brew services stop <formula>
restart:  brew services restart <formula>
run:      brew services run <formula>
kill:     brew services kill <formula>
cleanup:  brew services cleanup
info:     brew services info <formula> --json
```

Default action policy:

- If service source is `both`, prefer Homebrew lifecycle actions for Homebrew formula services.
- Use raw `launchctl` actions for custom plists.
- Require confirmation when the action needs `sudo`.
- Require stronger confirmation for `/Library/LaunchDaemons`.
- Block mutating actions for `/System/Library/*`.

### Smart Search And Filter

Search should match:

- display name
- launchd label
- plist path
- program path
- Homebrew formula
- tag
- health diagnostic text

Filters:

- source: launchd, homebrew, both
- status: running, stopped, failed, unloaded, disabled
- scope: user agents, global agents, system daemons, read-only system
- safety: user writable, admin required, protected
- health: has warnings, clean
- ownership: current user, root, vendor
- custom tags

Tags are local metadata stored by Launchdeck, not written into service plists.

### Service Logs

Log sources:

- `StandardOutPath`
- `StandardErrorPath`
- Homebrew service log paths when present in the plist
- `log stream --predicate ...` for live unified logs
- `log show --last ... --predicate ...` for recent history

Log view features:

- tail mode for stdout/stderr files
- live unified-log stream
- filter by text
- filter by level when unified log data exposes it
- pause/resume
- copy selected line
- jump to latest

The first implementation can start with file tails and a generated `log stream` command preview. Live unified-log streaming can follow once process/label predicates are reliable.

## Safety Rules

Launchdeck should classify every service before enabling mutating actions.

`readonly_system`:

- any plist below `/System/Library`
- allowed: inspect, search, logs when accessible
- blocked: edit, delete, load, unload, enable, disable, start, stop, restart

`protected_vendor`:

- known vendor plists in `/Library`
- allowed: inspect, logs, start/stop with confirmation
- blocked by default: edit plist, delete plist

`admin_required`:

- `/Library/LaunchAgents`
- `/Library/LaunchDaemons`
- allowed with confirmation: bootstrap, bootout, enable, disable, start, stop, restart
- requires sudo strategy

`user_writable`:

- `~/Library/LaunchAgents`
- allowed with normal confirmation for destructive or persistent changes

Destructive actions:

- deleting a plist
- disabling a service
- booting out a service
- overwriting an existing plist
- changing a root-owned service

These require a confirmation dialog that states the file path, domain, and exact command.

## Architecture

Recommended stack:

- Rust
- `ratatui` for UI
- `crossterm` for terminal events
- `plist` for plist parsing/writing
- `serde` and `serde_json` for Homebrew JSON and metadata
- `tokio` for async command execution and refresh loops
- `notify` for plist directory watches
- `portable-pty` only if interactive sudo becomes necessary later

Core modules:

```text
src/app.rs             event loop and top-level state
src/ui/                ratatui views and widgets
src/discovery/         launchd, brew, filesystem, metadata discovery
src/model.rs           unified service model
src/actions/           command planning and execution
src/plist_io.rs        plist parse, validate, write
src/logs.rs            file tail and unified log adapters
src/safety.rs          service classification and action gates
src/config.rs          app config and tags
```

Action execution should be two-phase:

1. Build an `ActionPlan` with command, required privileges, risks, and expected state change.
2. Ask for confirmation when needed, execute, capture stdout/stderr, refresh service state.

## Local Metadata

Launchdeck stores app-only metadata at:

```text
~/Library/Application Support/Launchdeck/config.json
```

Initial fields:

```json
{
  "refresh_interval_ms": 3000,
  "tags": {
    "homebrew.mxcl.redis": ["database", "dev"]
  },
  "hidden_services": [],
  "confirm_admin_actions": true
}
```

## Milestones

### Milestone 1: Read-Only Service Inventory

- Parse plist directories.
- Read `brew services list --json`.
- Merge Homebrew and launchd-backed services.
- Render service overview.
- Search and filter by source, status, and scope.
- Detail view with parsed plist fields.

### Milestone 2: Runtime Status

- Query `launchctl print gui/<uid>`, `launchctl print user/<uid>`, and `launchctl print system`.
- Resolve PID, exit code, loaded state, and disabled state.
- Add periodic refresh.
- Add health diagnostics.

### Milestone 3: Logs

- Detect stdout/stderr paths.
- Tail service log files.
- Add filter, pause, and jump-to-latest.
- Add generated unified-log commands in detail view.

### Milestone 4: Safe Lifecycle Actions

- Implement action planning.
- Implement start, stop, restart, enable, disable for user services.
- Implement Homebrew start, stop, restart, run, kill.
- Add admin-required action warnings without interactive sudo yet.

### Milestone 5: Service Creation

- Add form builder for user agents.
- Validate labels, paths, plist syntax, and output paths.
- Write plist to `~/Library/LaunchAgents`.
- Offer bootstrap and start after creation.

### Milestone 6: Admin Scope

- Add sudo strategy for `/Library/LaunchAgents` and `/Library/LaunchDaemons`.
- Add system daemon form builder.
- Keep `/System/Library/*` read-only.

## Open Questions

- Whether the first release should support interactive sudo inside the TUI or print a command for the user to run externally.
- How much of `launchctl print` output should be parsed directly versus treated as diagnostic text.
- Whether tags should key by launchd label only or by `(domain, label)` to avoid collisions.
- Whether Homebrew formula services should expose raw `launchctl` actions as advanced operations.
- Whether service creation should support plist import from an existing command first, before the full form builder.

## Verification Commands

These commands define the first local discovery target:

```sh
brew services list --json
brew services info --all --json
launchctl print gui/$(id -u)
launchctl print user/$(id -u)
launchctl print system
launchctl print-disabled gui/$(id -u)
fd -d 1 . ~/Library/LaunchAgents /Library/LaunchAgents /Library/LaunchDaemons
plutil -lint ~/Library/LaunchAgents/*.plist
```
