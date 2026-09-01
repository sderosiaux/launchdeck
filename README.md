# Launchdeck

[![CI](https://github.com/sderosiaux/launchdeck/actions/workflows/ci.yml/badge.svg)](https://github.com/sderosiaux/launchdeck/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/sderosiaux/launchdeck?sort=semver)](https://github.com/sderosiaux/launchdeck/releases)
[![crates.io](https://img.shields.io/crates/v/launchdeck)](https://crates.io/crates/launchdeck)
[![macOS](https://img.shields.io/badge/platform-macOS-lightgrey)](#requirements)
[![Rust 2024](https://img.shields.io/badge/Rust-2024-orange)](Cargo.toml)

**Your Mac is running things you never agreed to.**

Updaters for apps you deleted. Jobs that wake up while you sleep. A service that
died months ago and never told you. Launchdeck puts all of it on one screen — and
lets you shut it down for good.

**[sderosiaux.github.io/launchdeck](https://sderosiaux.github.io/launchdeck/)**

![Launchdeck TUI](assets/launchdeck.gif)

## Why Launchdeck?

Background work on macOS is spread across `launchd`, Homebrew and a pile of plist
files nobody reads, and the day-to-day experience is fragmented:

- `launchctl` knows runtime state, but its output is dense and domain-oriented,
  and it says nothing about the plist that defines the job.
- Plists explain schedules, logs, `RunAtLoad` and `KeepAlive` — but you have to
  go find them first.
- `brew services` is convenient, and blind to everything that is not a formula.
- A stopped process might actually be a scheduled job waiting for its next run.
  Those look identical from the outside.
- Killing a process is not the same thing as unloading a launchd job. It comes
  back.

Launchdeck brings those pieces together and adds the safety rails you want before
changing anything. It shows the command before it runs, blocks risky
system/vendor actions, preserves selection by service identity across refreshes,
and makes logs and schedules visible where you make decisions.

## Install

Install the latest macOS release binary:

```sh
brew install sderosiaux/tap/launchdeck
```

Without Homebrew:

```sh
curl -fsSL https://raw.githubusercontent.com/sderosiaux/launchdeck/main/scripts/install.sh | sh
```

The installer writes to `~/.local/bin` by default. Override it with:

```sh
BIN_DIR=/usr/local/bin sh -c "$(curl -fsSL https://raw.githubusercontent.com/sderosiaux/launchdeck/main/scripts/install.sh)"
```

Install from source with Cargo:

```sh
cargo install launchdeck
```

Install from the GitHub repository:

```sh
cargo install --git https://github.com/sderosiaux/launchdeck
```

Or build locally:

```sh
git clone https://github.com/sderosiaux/launchdeck.git
cd launchdeck
cargo build --release
./target/release/launchdeck
```

## Usage

```sh
launchdeck
```

Print inventory without opening the TUI:

```sh
launchdeck list
```

`launchdeck list` uses the same default visibility as the TUI and hides Apple/system services. Print the full discovered inventory with:

```sh
launchdeck list --all
```

`launchdeck --version` prints the version, `launchdeck --help` the usage summary.

## Features

- One service list for `launchd` jobs and Homebrew services.
- Runtime state from `launchctl`, enriched with Homebrew metadata and parsed plist configuration.
- A distinct `scheduled` state for loaded jobs that are not running now but will wake up later.
- Compact schedule summaries such as `5min`, `1h`, `00:00`, and `Sun 09:00`.
- Type-to-search, source/status filters, warnings-only view, Apple/system toggle, and practical sorting.
- Provenance for every job: which tool installed it (`homebrew`, `user-plist`, `mise`, `nix`, `vendor-app`, `system`, `runtime-only`), with the evidence behind the guess.
- Detail modal for status, scope, safety level, origin, elevation, command, plist path, schedule, logs, and health warnings.
- Scrollable stdout/stderr log view from configured `StandardOutPath` and `StandardErrorPath`.
- Confirmed actions for start, stop, restart/load, enable/disable, `RunAtLoad`, edit plist, and delete plist.
- User LaunchAgent creation for common plist fields without hand-writing XML.
- Background refresh that keeps the selected service stable by identity, not row index.

## Status Model

Launchdeck separates states that can otherwise look the same in `launchctl` output:

| Status | Meaning |
| --- | --- |
| `running` | launchd has an active PID for the job. |
| `scheduled` | job is loaded, has no active PID, and has a launchd schedule configured. |
| `stopped` | job is loaded, has no active PID, and has no known schedule. |
| `unloaded` | plist exists, but the job is not loaded into launchd. |
| `disabled` | launchd marks the job disabled in its domain. |
| `failed` | launchd or Homebrew reported a non-zero exit/error state. |
| `unknown` | Launchdeck could not classify the current state. |

For scheduled jobs, `stop` uses `launchctl bootout` so the job is actually unloaded and will not wake up on the next schedule.

## Keybindings

### Overview

| Key | Action |
| --- | --- |
| Type text | Start quick search |
| `Down` / `Up` | Move selection |
| `PageDown` / `PageUp` | Move by a page |
| `Enter` | Open service detail |
| `?` / `F1` | Open keyboard help |
| `/` | Search |
| `C` | Clear search |
| `Y` | Copy selected service name |
| `P` | Cycle source filter |
| `F` | Cycle status filter |
| `O` | Cycle sort mode |
| `A` | Toggle Apple/system services |
| `W` | Toggle warnings-only view |
| `F5` / `Ctrl-r` | Refresh inventory |
| `L` | Open logs |
| `S` | Prepare start action |
| `X` | Prepare stop action |
| `R` | Prepare restart/load action |
| `T` | Prepare enable/disable action |
| `U` | Prepare `RunAtLoad` toggle action |
| `E` | Prepare edit plist action |
| `D` | Prepare delete plist action |
| `N` | Create a user LaunchAgent |
| `q` | Quit |

Actions show the exact command before execution. Press `y`/`Enter` to confirm or `n`/`Esc` to cancel.

### Detail

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down inside detail |
| `k` / `Up` | Move up inside detail |
| `PageDown` / `PageUp` | Move by a page |
| `g` / `G` | First / last detail row |
| `Enter` | Act on selected row: status, plist, `RunAtLoad`, stdout, or stderr |
| `c` | Copy selected field value |
| `l` | Open stdout logs |
| `u` | Prepare `RunAtLoad` toggle action |
| `E` | Prepare edit plist action |
| `D` | Prepare delete plist action |
| `Esc` / `Backspace` / `Left` | Back to overview |

### Logs

| Key | Action |
| --- | --- |
| `j` / `Down` | Newer lines |
| `k` / `Up` | Older lines |
| `PageDown` / `PageUp` | Move by a page |
| `g` / `G` | Top / bottom of loaded tail |
| `Tab` / `Left` / `Right` | Switch stdout/stderr |
| `c` | Copy current log path |
| `Esc` / `Backspace` | Back to detail |

The log view opens at the end of the selected stream and keeps a scrollback window from the latest 500 lines.

## Safety

Launchdeck is conservative by default:

- Homebrew services are managed through `brew services`.
- User-owned launchd jobs are managed through `launchctl`.
- Services under `/System/Library` are inspect-only.
- Vendor/runtime services are blocked unless they can be classified safely.
- Destructive actions, including delete, always require confirmation.

Privileges are decided per action, not per service, because launchd scope and file
ownership are independent. An agent installed in `/Library/LaunchAgents` is
root-owned on disk but runs in your own `gui/<uid>` domain: starting and stopping
it needs nothing, while editing or deleting its plist needs root. The detail view
shows exactly which axes need elevation.

When an action does need root, Launchdeck prefixes it with `sudo`, shows the full
command including that prefix, then leaves the TUI so `sudo` can prompt on the real
terminal. The password never passes through Launchdeck, and the command is always
executed as an argument list, never through a shell.

## Create Form

The create form writes user LaunchAgents only, under `~/Library/LaunchAgents`.

It supports common plist fields: label, program arguments, working directory, stdout/stderr paths, environment variables, `RunAtLoad`, `KeepAlive`, `StartInterval`, optional bootstrap, and optional start.

Arguments and environment values accept shell-style quotes, so values like `--name "hello world"` are preserved correctly.

## Requirements

- macOS
- Homebrew optional, for `brew services` support
- Rust toolchain only when installing from source

## Test Fixture

The repository includes a fake user agent plist for local discovery testing:

```sh
cp fixtures/com.sderosiaux.launchdeck.fake.plist ~/Library/LaunchAgents/
cargo run -- --list | rg launchdeck.fake
```

The fixture is not loaded or started by that command. Remove it with:

```sh
rm ~/Library/LaunchAgents/com.sderosiaux.launchdeck.fake.plist
```

## Development

```sh
scripts/lint.sh
cargo run
cargo run -- --list
```

## Release

Releases are built from tags:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The release workflow builds macOS archives for Apple Silicon and Intel, publishes checksums, and updates the GitHub release assets.

## Project Status

Launchdeck is early software. It is already useful for inventory, inspection, filtering, navigable logs, guarded lifecycle actions, and creating user LaunchAgents.
