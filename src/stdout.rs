use crate::model::PackageUpdate;
use crate::size::parse_dnf_size;
use anyhow::{Context, bail};

pub fn parse_update_lines(stdout: &str) -> anyhow::Result<Vec<PackageUpdate>> {
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
                bail!(
                    "'replacing' line for '{name}' has {} fields, expected ≥4: {line:?}",
                    parts.len()
                );
            }
            if parts[0] != name {
                bail!(
                    "'replacing' references '{}' but expected '{name}'",
                    parts[0]
                );
            }
            let u = updates
                .last_mut()
                .expect("updates non-empty when pending is set");
            u.old_version = normalize_version(parts[2]);
            u.old_repo = parts[3].to_string();
        } else {
            if let Some(ref name) = pending {
                bail!(
                    "expected 'replacing' line for '{name}' but got another package line: {line:?}"
                );
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 6 {
                bail!(
                    "package line has {} fields, expected 6: {line:?}",
                    parts.len()
                );
            }
            pending = Some(parts[0].to_string());
            updates.push(PackageUpdate {
                name: parts[0].to_string(),
                arch: parts[1].to_string(),
                new_version: normalize_version(parts[2]),
                old_version: String::new(),
                old_repo: String::new(),
                new_repo: parts[3].to_string(),
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
