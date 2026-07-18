use crate::model::SizeInfo;
use crate::size::parse_dnf_size;
use anyhow::Context;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::BufRead;
use std::time::Duration;

pub fn process_stderr(stderr: impl std::io::Read) -> anyhow::Result<SizeInfo> {
    let reader = std::io::BufReader::new(stderr);
    let mut size_info = SizeInfo::default();
    let mut spinner: Option<ProgressBar> = None;

    for line in reader.lines() {
        let line = line.context("reading dnf stderr")?;
        match line.as_str() {
            "Updating and loading repositories:" => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
                        .template("{spinner:.cyan} {msg}")
                        .unwrap(),
                );
                pb.enable_steady_tick(Duration::from_millis(100));
                pb.set_message("Updating and loading repositories...");
                spinner = Some(pb);
            }
            "Repositories loaded." => {
                if let Some(pb) = spinner.take() {
                    pb.finish_and_clear();
                }
            }
            "Operation aborted by the user." => {}
            s if s.starts_with("Total size of inbound packages is") => {
                size_info.download = parse_download_line(s);
            }
            s if s.starts_with("After this operation,") => {
                size_info.net_disk = parse_disk_line(s);
            }
            other => match &spinner {
                Some(pb) => pb.println(other),
                None => eprintln!("{other}"),
            },
        }
    }

    if let Some(pb) = spinner.take() {
        pb.finish_and_clear();
    }

    Ok(size_info)
}

fn parse_download_line(line: &str) -> Option<u64> {
    // "Total size of inbound packages is 53 MiB. Need to download 53 MiB."
    let need_part = line.split(". ").nth(1)?;
    let need_part = need_part.trim_end_matches('.');
    let words: Vec<&str> = need_part.split_whitespace().collect();
    if words.len() == 5 && words[..3] == ["Need", "to", "download"] {
        parse_dnf_size(words[3], words[4]).ok()
    } else {
        None
    }
}

fn parse_disk_line(line: &str) -> Option<i64> {
    // "After this operation, 11 MiB extra will be used (install 275 MiB, remove 264 MiB)."
    // "After this operation, 5 MiB will be freed (install 264 MiB, remove 269 MiB)."
    let rest = line.strip_prefix("After this operation, ")?;
    let words: Vec<&str> = rest.split_whitespace().collect();
    if words.len() < 4 {
        return None;
    }
    let bytes = parse_dnf_size(words[0], words[1]).ok()? as i64;
    match &words[2..4] {
        ["extra", "will"] => Some(bytes),
        ["will", "be"] if words.get(4) == Some(&"freed") => Some(-bytes),
        _ => None,
    }
}
