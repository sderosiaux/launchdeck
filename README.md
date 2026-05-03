# Launchdeck

[![CI](https://github.com/sderosiaux/launchdeck/actions/workflows/ci.yml/badge.svg)](https://github.com/sderosiaux/launchdeck/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/sderosiaux/launchdeck?sort=semver)](https://github.com/sderosiaux/launchdeck/releases)
[![macOS](https://img.shields.io/badge/platform-macOS-lightgrey)](#requirements)
[![Rust 2024](https://img.shields.io/badge/Rust-2024-orange)](Cargo.toml)

Keyboard-first macOS TUI for inspecting and managing `launchd` jobs and Homebrew services from one place.

Launchdeck is built for developer machines where background work is split between raw plist files in `~/Library/LaunchAgents` and services managed by `brew services`.

![Launchdeck TUI](assets/launchdeck.gif)

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

## Features

- Unified service list for `launchd` jobs and Homebrew services.
- Runtime status from `launchctl`, plus Homebrew metadata from `brew services list --json`.
- `scheduled` status for loaded launchd jobs waiting for their next `StartInterval` or `StartCalendarInterval`.
- Compact schedule summaries such as `5min`, `1h`, `00:00`, or `Sun 09:00`.
- Type-to-search on the main screen, with filters for source, status, Apple/system services, warnings, and sorting.
- Detail modal with colored fields for status, command, plist path, schedule, logs, and health warnings.
- Scrollable stdout/stderr log view from configured `StandardOutPath` and `StandardErrorPath`.
- Guarded actions for start, stop, restart/load, enable/disable, `RunAtLoad`, edit plist, and delete plist.
- User LaunchAgent creation form for common plist fields.
- Background refresh that preserves the selected service by identity, not row index.

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
- Admin-required services are blocked until sudo handling is implemented.
- Vendor/runtime services are blocked unless they can be classified safely.
- Destructive actions, including delete, always require confirmation.

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

Launchdeck is early software. It is already useful for inventory, inspection, filtering, navigable logs, guarded lifecycle actions, and creating user LaunchAgents. Sudo-backed admin actions are not implemented yet.
