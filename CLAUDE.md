# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`fnf` (Fancified YUM) is a `dnf` wrapper that enhances `dnf upgrade` with yay-style colored output: version diffs highlighted by differing segment, aligned columns, download sizes, and a Y/n confirmation prompt before running the actual upgrade.

Binary name: `fnf`. The repo directory is `dnf-wrapper` for historical reasons.

## Commands

```sh
cargo build                    # build debug binary → target/debug/fnf
cargo run -- upgrade           # run (aliases: up, update)
cargo clippy                   # lint
cargo test                     # run tests (none currently)
```

Manual testing requires `dnf` and `rpm` on the system:

```sh
target/debug/fnf upgrade       # runs dnf check-update, then prompts
```

## Architecture

Everything lives in `src/main.rs`. No modules, no workspace.

**Upgrade flow:**

1. `check_updates()` — runs `dnf check-update --color=never`; exit code 100 means updates are available (dnf convention), 0 means up to date
2. Output is parsed by `parse_update_line()`, which splits on whitespace and validates arch against `KNOWN_ARCHES` to skip header/blank lines
3. Installed versions are queried via `rpm -qa` with epoch:version-release format; stored in a `HashMap<String, String>` keyed as `name.arch`
4. Download sizes are batch-fetched via `dnf repoquery --queryformat`
5. `display_updates()` prints an aligned table; `highlight_version_diff()` finds the longest common prefix and suffix between old/new version strings and colors only the differing middle segment
6. After Y/n confirmation, `do_upgrade()` exec-replaces into `dnf upgrade -y`

**Key details:**
- `normalize_version()` strips the `0:` epoch prefix that `rpm` includes
- Column widths are computed from max lengths before printing, so all rows align
- `DNF` constant points to `/usr/bin/dnf` (absolute path)
