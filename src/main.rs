mod cmd;
mod model;
mod size;
mod stderr;
mod stdout;

use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::cmd::Cmd;
use crate::model::{PackageUpdate, SizeInfo};
use crate::stderr::process_stderr;
use crate::stdout::parse_update_lines;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;

const DNF: &str = "/usr/bin/dnf";
const LOCALE_ENV: (&str, &str) = ("LC_ALL", "C.UTF-8");

#[derive(Parser)]
#[command(
    name = "fnf",
    about = "Fancified YUM — dnf wrapper with improved upgrade output"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(alias = "up", alias = "update", about = "Upgrade all packages")]
    Upgrade {
        #[arg(long, short = 'a', help = "Show architecture column")]
        show_arch: bool,
        #[arg(long, short = 'c', help = "Print the dnf command before running it")]
        show_command: bool,
        #[arg(long, short = 'g', value_enum, default_value_t = GroupBy::Repository, help = "Group packages")]
        group: GroupBy,
    },
    #[command(about = "Refresh metadata for all enabled repositories")]
    Refresh,
    #[command(alias = "clean-all", about = "Remove all cached DNF repository data")]
    Clean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum GroupBy {
    /// Group packages by repository
    Repository,
    /// Do not group packages
    None,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Upgrade {
            show_arch,
            show_command,
            group,
        } => run_upgrade_wrapper(&Options {
            show_arch,
            show_command,
            group,
        }),
        Commands::Refresh => run_dnf_command(refresh_cmd()),
        Commands::Clean => run_dnf_command(clean_cmd()),
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

struct Options {
    show_arch: bool,
    show_command: bool,
    group: GroupBy,
}

fn run_upgrade_wrapper(options: &Options) -> Result<()> {
    let (updates, size_info) = check_updates().context("checking for updates")?;

    if updates.is_empty() {
        println!("{}", " :: System is up to date.".green().bold());
        return Ok(());
    }

    let Options {
        show_arch,
        show_command,
        group,
    } = *options;

    display_updates(&updates, show_arch, group, &size_info);

    let upgrade_cmd = upgrade_cmd(&updates);

    if show_command {
        println!("{}", format!("\n==> Command: {upgrade_cmd}").dimmed());
    }

    print!("\n{} ", "==> Proceed with upgrade? [Y/n]".bold());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_lowercase();

    if answer.is_empty() || answer == "y" || answer == "yes" {
        let exit_code = upgrade_cmd.execute()?;
        std::process::exit(exit_code);
    } else {
        println!("{}", "Operation cancelled.".yellow());
    }

    Ok(())
}

fn check_updates() -> Result<(Vec<PackageUpdate>, SizeInfo)> {
    let mut cmd = dnf_cmd();
    cmd.args(["upgrade", "--assumeno", "--color=never"]);
    let mut child = Command::from(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running dnf upgrade --assumeno")?;

    let stderr = child.stderr.take().expect("stderr is piped");
    let stderr_thread = std::thread::spawn(move || process_stderr(stderr));

    let output = child
        .wait_with_output()
        .context("waiting for dnf upgrade --assumeno")?;

    let size_info = stderr_thread
        .join()
        .expect("stderr thread panicked")
        .context("processing dnf stderr")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let updates = parse_update_lines(&stdout).context("parsing dnf output")?;

    Ok((updates, size_info))
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KiB", bytes as f64 / (1u64 << 10) as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn highlight_diff(old: &str, new: &str) -> (String, String) {
    let prefix_len = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();

    let old_rest = &old[prefix_len..];
    let new_rest = &new[prefix_len..];

    let suffix_len = old_rest
        .bytes()
        .rev()
        .zip(new_rest.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let prefix = &old[..prefix_len];
    let old_mid = &old_rest[..old_rest.len() - suffix_len];
    let new_mid = &new_rest[..new_rest.len() - suffix_len];
    let suffix = &old_rest[old_rest.len() - suffix_len..];

    let old_str = format!(
        "{}{}{}",
        prefix.dimmed(),
        old_mid.red().bold(),
        suffix.dimmed()
    );
    let new_str = format!(
        "{}{}{}",
        prefix.dimmed(),
        new_mid.green().bold(),
        suffix.dimmed()
    );

    (old_str, new_str)
}

fn shorten_repo(repo: &str) -> String {
    if repo.len() >= 20 && repo.bytes().all(|b| b.is_ascii_hexdigit()) {
        format!("{}..{}", &repo[..2], &repo[repo.len() - 2..])
    } else {
        repo.to_string()
    }
}

fn display_updates(
    updates: &[PackageUpdate],
    show_arch: bool,
    group: GroupBy,
    size_info: &SizeInfo,
) {
    let count = updates.len();

    let size_str = match (size_info.download, size_info.net_disk) {
        (Some(dl), Some(disk)) => {
            let disk_str = if disk >= 0 {
                format!("+{}", format_size(disk as u64))
            } else {
                format!("-{}", format_size((-disk) as u64))
            };
            format!("{} download, {} disk", format_size(dl), disk_str)
        }
        (Some(dl), None) => format_size(dl),
        (None, Some(disk)) => {
            if disk >= 0 {
                format!("+{} disk", format_size(disk as u64))
            } else {
                format!("-{} disk", format_size((-disk) as u64))
            }
        }
        (None, None) => {
            let total: u64 = updates.iter().map(|u| u.download_size).sum();
            format_size(total)
        }
    };

    println!(
        "{}",
        format!(
            " :: {} package{} to upgrade  ({})",
            count,
            if count == 1 { "" } else { "s" },
            size_str,
        )
        .cyan()
        .bold()
    );
    println!();

    let max_name = updates.iter().map(|u| u.name.len()).max().unwrap_or(0);
    let max_arch = updates.iter().map(|u| u.arch.len()).max().unwrap_or(0);
    let max_old = updates
        .iter()
        .map(|u| u.old_version.len())
        .max()
        .unwrap_or(0);
    let max_new = updates
        .iter()
        .map(|u| u.new_version.len())
        .max()
        .unwrap_or(0);
    let max_size = updates
        .iter()
        .map(|u| format_size(u.download_size).len())
        .max()
        .unwrap_or(0);

    let print_row = |update: &PackageUpdate| {
        let (old_ver, new_ver) = highlight_diff(&update.old_version, &update.new_version);

        let name_padded = format!("{:<max_name$}", update.name);
        let old_pad = " ".repeat(max_old.saturating_sub(update.old_version.len()));
        let new_pad = " ".repeat(max_new.saturating_sub(update.new_version.len()));
        let size_str = format_size(update.download_size);
        let size_pad = " ".repeat(max_size.saturating_sub(size_str.len()));

        let old_repo = shorten_repo(&update.old_repo);
        let new_repo = shorten_repo(&update.new_repo);
        let repo_display = if update.old_repo.is_empty() || update.old_repo == update.new_repo {
            new_repo.dimmed().to_string()
        } else {
            let (old_r, new_r) = highlight_diff(&old_repo, &new_repo);
            format!("{} -> {}", old_r, new_r)
        };

        let arch_col = if show_arch {
            format!("  {}", format!("{:<max_arch$}", update.arch).dimmed())
        } else {
            String::new()
        };

        println!(
            "    {}{}  {}{} -> {}{}  {}{}  {}",
            name_padded.bold(),
            arch_col,
            old_ver,
            old_pad,
            new_ver,
            new_pad,
            size_pad,
            size_str.dimmed(),
            repo_display,
        );
    };

    match group {
        GroupBy::None => {
            for update in updates {
                print_row(update);
            }
        }
        GroupBy::Repository => {
            let mut order: Vec<&PackageUpdate> = updates.iter().collect();
            order.sort_by(|a, b| {
                a.new_repo
                    .cmp(&b.new_repo)
                    .then_with(|| a.name.cmp(&b.name))
            });
            let mut current: Option<&str> = None;
            for update in order {
                if current != Some(update.new_repo.as_str()) {
                    if current.is_some() {
                        println!();
                    }
                    current = Some(update.new_repo.as_str());
                    println!("  {}", shorten_repo(&update.new_repo).underline().bold());
                }
                print_row(update);
            }
        }
    }
}

fn run_dnf_command(cmd: Cmd) -> Result<()> {
    let command = cmd.to_string();
    let exit_code = cmd
        .execute()
        .with_context(|| format!("running {command}"))?;

    if exit_code != 0 {
        anyhow::bail!("{command} exited with status {exit_code}");
    }

    Ok(())
}

fn refresh_cmd() -> Cmd {
    let mut cmd = dnf_cmd();
    cmd.args(["--refresh", "makecache"]);
    cmd
}

fn clean_cmd() -> Cmd {
    let mut cmd = dnf_cmd();
    cmd.args(["clean", "all"]);
    cmd
}

fn upgrade_specs(updates: &[PackageUpdate]) -> Vec<String> {
    // name-[epoch:]version-release.arch — pins dnf to exactly what was displayed
    updates
        .iter()
        .map(|u| format!("{}-{}.{}", u.name, u.new_version, u.arch))
        .collect()
}

fn upgrade_cmd(updates: &[PackageUpdate]) -> Cmd {
    let mut cmd = dnf_cmd();
    cmd.arg("upgrade").arg("-y").args(upgrade_specs(updates));
    cmd
}

fn dnf_cmd() -> Cmd {
    let mut cmd = Cmd::new(DNF);
    let (k, v) = LOCALE_ENV;
    cmd.env(k, v);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_command_forces_repository_metadata_refresh() {
        assert_eq!(
            refresh_cmd().to_string(),
            "LC_ALL=C.UTF-8 /usr/bin/dnf --refresh makecache"
        );
    }

    #[test]
    fn clean_command_removes_all_cached_dnf_data() {
        assert_eq!(
            clean_cmd().to_string(),
            "LC_ALL=C.UTF-8 /usr/bin/dnf clean all"
        );
    }
}
