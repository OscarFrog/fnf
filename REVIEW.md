# Code Review — fnf (src/main.rs)

Findings ranked by severity. Correctness bugs first, then efficiency, cleanup, and docs.

## Findings

### 1. dnf error silently becomes "up to date" — `main.rs:84`

`check_updates()` never checks `output.status`. If dnf fails (RPM lock held by PackageKit, network error, permission denied), it exits nonzero with error text on stderr but empty stdout. The function returns `Ok(vec![])` and the tool prints "System is up to date." — a silent false negative.

Nuance: `dnf upgrade --assumeno` also exits 1 when there *are* updates (it would have made changes), so a simple nonzero check isn't enough. The fix needs to distinguish "nothing to do" (zero exit, empty parse) from "genuine error" (nonzero exit, empty parse result).

### 2. `do_upgrade()` panics instead of returning `Result` — `main.rs:268`

`do_upgrade()` uses `.expect()` and calls `std::process::exit()` directly, bypassing the anyhow error chain used everywhere else. If `/usr/bin/dnf` cannot be executed at upgrade time (binary missing, permissions changed between check and upgrade), `Command::status()` returns `Err` which `.expect()` turns into a panic with a raw Rust backtrace instead of the clean `Error: ...` message `main()` produces for other failures.

### 3. True package obsoletes silently dropped — `main.rs:102`

For true package replacements (old-name → new-name, e.g. `python-foo` → `python3-foo`), the `replacing` sub-line's `parts[0]` is the **old** package name, but the HashMap key was inserted as the **new** package name. `get_mut(parts[0])` returns `None`, `old_version` stays empty, and the entry is dropped by the `!u.old_version.is_empty()` filter at line 132 with no warning.

Normal in-place upgrades (same package name, new version) are unaffected because `parts[0]` on the replacing line matches the HashMap key.

### 4. `format_size` called twice per package — `main.rs:225`

`display_updates()` calls `format_size(u.download_size)` once per package in the `max_size` width-scan pass (line 225) and again in the print loop (line 235), allocating two identical `String`s per row. Fix: collect formatted size strings into a `Vec<String>` during the width pass and index into it in the print loop.

### 5. `shorten_repo` called before equality check — `main.rs:238`

`shorten_repo()` is called on both `old_repo` and `new_repo` before the `old_repo == repo` equality check. In the common case where the repo is unchanged, both calls run and their results are discarded. Moving the equality check before the `shorten_repo` calls eliminates the wasted allocations on the common path.

### 6. `chars()` instead of `bytes()` for ASCII predicate — `main.rs:195`

`shorten_repo()` uses `repo.chars().all(|c| c.is_ascii_hexdigit())` which unnecessarily UTF-8 decodes each character for a predicate that only cares about byte values. `repo.bytes().all(|b| b.is_ascii_hexdigit())` is equivalent and idiomatic for ASCII-only validation.

### 7. Duplicate `String` allocation for HashMap key — `main.rs:113`

`parts[0].to_string()` is called twice: once for the HashMap key (line 113) and once for `PackageUpdate.name` (line 117). The key is discarded by `into_values()`, so only one allocation is needed. Fix: `let name = parts[0].to_string(); updates.insert(name.clone(), PackageUpdate { name, ... })` or use `entry()`.

### 8. `max_arch` computed when `show_arch` is false — `main.rs:220`

The `max_arch` column width is computed by iterating all updates even when `show_arch` is `false` and the result is never used. Move the computation inside the `show_arch` branch or guard it with `if show_arch { ... }`.

### 9. Inline comment on `shorten_repo` duplicates CLAUDE.md — `main.rs:193`

The comment at lines 193–194 repeats the CLAUDE.md line 48 description of `shorten_repo()` verbatim. Two copies will drift independently when the logic changes. Remove the inline comment; CLAUDE.md is the authoritative source for architectural prose.

---

## Summary Table

| # | Severity    | Location      | Issue                                              |
|---|-------------|---------------|----------------------------------------------------|
| 1 | Bug         | `main.rs:84`  | dnf error silently becomes "up to date"            |
| 2 | Bug         | `main.rs:268` | `do_upgrade()` panics instead of clean error       |
| 3 | Bug         | `main.rs:102` | True package obsoletes (old-name→new-name) dropped |
| 4 | Efficiency  | `main.rs:225` | `format_size` called twice per package             |
| 5 | Efficiency  | `main.rs:238` | `shorten_repo` called before equality check        |
| 6 | Idiom       | `main.rs:195` | `chars()` instead of `bytes()` for ASCII predicate |
| 7 | Cleanup     | `main.rs:113` | Duplicate `String` allocation for HashMap key      |
| 8 | Cleanup     | `main.rs:220` | `max_arch` computed even when `show_arch` is false |
| 9 | Docs        | `main.rs:193` | Inline comment duplicates CLAUDE.md verbatim       |
