use std::io::{self, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;

#[derive(Debug)]
struct PackageUpdate {
    name: String,
    arch: String,
    old_version: String,
    new_version: String,
    old_repo: String,
    repo: String,
    download_size: u64,
}

const DNF: &str = "/usr/bin/dnf";

#[derive(Parser)]
#[command(name = "fnf", about = "Fancified YUM — dnf wrapper with improved upgrade output")]
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
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Upgrade { show_arch } => run_upgrade_wrapper(show_arch),
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run_upgrade_wrapper(show_arch: bool) -> Result<()> {
    let updates = check_updates().context("checking for updates")?;

    if updates.is_empty() {
        println!("{}", " :: System is up to date.".green().bold());
        return Ok(());
    }

    display_updates(&updates, show_arch);

    print!("\n{} ", "==> Proceed with upgrade? [Y/n]".bold());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim().to_lowercase();

    if answer.is_empty() || answer == "y" || answer == "yes" {
        do_upgrade();
    } else {
        println!("{}", "Operation cancelled.".yellow());
    }

    Ok(())
}

fn check_updates() -> Result<Vec<PackageUpdate>> {
    let output = Command::new(DNF)
        .args(["upgrade", "--assumeno", "--color=never"])
        .stderr(Stdio::inherit())
        .output()
        .context("running dnf upgrade --assumeno")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_update_lines(&stdout).context("parsing dnf output")
}

fn parse_update_lines(stdout: &str) -> Result<Vec<PackageUpdate>> {
    let mut updates: Vec<PackageUpdate> = Vec::new();
    let mut in_upgrading = false;
    // Name of the last package line parsed, waiting for its `replacing` sub-line.
    let mut pending: Option<String> = None;

    for line in stdout.lines() {
        if !line.starts_with(' ') {
            if let Some(ref name) = pending {
                bail!("expected 'replacing' line for '{name}' but section ended");
            }
            in_upgrading = line.trim_end() == "Upgrading:";
            continue;
        }

        if !in_upgrading {
            continue;
        }

        if let Some(rest) = line.strip_prefix("   replacing ") {
            let name = pending.take().ok_or_else(|| {
                anyhow::anyhow!("unexpected 'replacing' line with no preceding package: {line:?}")
            })?;
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() < 4 {
                bail!("'replacing' line for '{name}' has {} fields, expected ≥4: {line:?}", parts.len());
            }
            if parts[0] != name {
                bail!("'replacing' references '{}' but expected '{name}'", parts[0]);
            }
            let u = updates.last_mut().expect("updates non-empty when pending is set");
            u.old_version = normalize_version(parts[2]);
            u.old_repo = parts[3].to_string();
        } else {
            if let Some(ref name) = pending {
                bail!("expected 'replacing' line for '{name}' but got another package line: {line:?}");
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 6 {
                bail!("package line has {} fields, expected 6: {line:?}", parts.len());
            }
            pending = Some(parts[0].to_string());
            updates.push(PackageUpdate {
                name: parts[0].to_string(),
                arch: parts[1].to_string(),
                new_version: normalize_version(parts[2]),
                old_version: String::new(),
                old_repo: String::new(),
                repo: parts[3].to_string(),
                download_size: parse_dnf_size(parts[4], parts[5])
                    .with_context(|| format!("parsing size on line {line:?}"))?,
            });
        }
    }

    if let Some(name) = pending {
        bail!("expected 'replacing' line for '{name}' but output ended");
    }

    updates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(updates)
}

fn normalize_version(v: &str) -> String {
    v.strip_prefix("0:").unwrap_or(v).to_string()
}

fn parse_dnf_size(number: &str, unit: &str) -> Result<u64> {
    let n: f64 = number.parse().with_context(|| format!("invalid size number: {number:?}"))?;
    Ok(match unit {
        "GiB" => (n * (1u64 << 30) as f64) as u64,
        "MiB" => (n * (1u64 << 20) as f64) as u64,
        "KiB" => (n * (1u64 << 10) as f64) as u64,
        "B" => n as u64,
        _ => bail!("unknown size unit: {unit:?}"),
    })
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

    let old_str = format!("{}{}{}", prefix.dimmed(), old_mid.red().bold(), suffix.dimmed());
    let new_str = format!("{}{}{}", prefix.dimmed(), new_mid.green().bold(), suffix.dimmed());

    (old_str, new_str)
}

fn shorten_repo(repo: &str) -> String {
    // Hex transaction hashes (e.g. 19278be6a81040f5b6cbc7bacea5148e) are replaced
    // with a short form like "19..8e" since they carry no useful meaning.
    if repo.len() >= 20 && repo.chars().all(|c| c.is_ascii_hexdigit()) {
        format!("{}..{}", &repo[..2], &repo[repo.len() - 2..])
    } else {
        repo.to_string()
    }
}

fn display_updates(updates: &[PackageUpdate], show_arch: bool) {
    let count = updates.len();
    let total_size = updates.iter().map(|u| u.download_size).sum();

    println!(
        "{}",
        format!(
            " :: {} package{} to upgrade  ({})",
            count,
            if count == 1 { "" } else { "s" },
            format_size(total_size),
        )
        .cyan()
        .bold()
    );
    println!();

    let max_name = updates.iter().map(|u| u.name.len()).max().unwrap_or(0);
    let max_arch = updates.iter().map(|u| u.arch.len()).max().unwrap_or(0);
    let max_old = updates.iter().map(|u| u.old_version.len()).max().unwrap_or(0);
    let max_new = updates.iter().map(|u| u.new_version.len()).max().unwrap_or(0);
    let max_size = updates
        .iter()
        .map(|u| format_size(u.download_size).len())
        .max()
        .unwrap_or(0);

    for update in updates {
        let (old_ver, new_ver) = highlight_diff(&update.old_version, &update.new_version);

        let name_padded = format!("{:<max_name$}", update.name);
        let old_pad = " ".repeat(max_old.saturating_sub(update.old_version.len()));
        let new_pad = " ".repeat(max_new.saturating_sub(update.new_version.len()));
        let size_str = format_size(update.download_size);
        let size_pad = " ".repeat(max_size.saturating_sub(size_str.len()));

        let old_repo = shorten_repo(&update.old_repo);
        let new_repo = shorten_repo(&update.repo);
        let repo_display = if update.old_repo.is_empty() || update.old_repo == update.repo {
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
    }
}

fn do_upgrade() {
    let status = Command::new(DNF)
        .args(["upgrade", "-y"])
        .status()
        .expect("failed to run dnf upgrade");

    std::process::exit(status.code().unwrap_or(1));
}
