use crate::model::PackageUpdate;
use crate::size::parse_dnf_size;
use anyhow::{Context, bail};
use std::collections::BTreeMap;

/// Strict state machine over the entire `dnf upgrade` stdout transaction table.
///
/// Every line must match the pattern the current state expects; any deviation —
/// unknown line shape, wrong field count, a missing/orphan `replacing` sub-line,
/// an unknown section header, truncated output, or a Transaction Summary count
/// that disagrees with the sections actually parsed — is a hard error. This
/// surfaces a change in dnf's output format immediately instead of silently
/// misbehaving.
pub fn parse_update_lines(stdout: &str) -> anyhow::Result<Vec<PackageUpdate>> {
    let mut parser = TableParser::new();
    for (idx, line) in stdout.lines().enumerate() {
        parser
            .feed(line)
            .with_context(|| format!("at stdout line {}: {line:?}", idx + 1))?;
    }
    parser.finish()
}

struct TableParser {
    state: State,
    updates: Vec<PackageUpdate>,
    /// Packages seen per summary bucket, keyed by `summary_group` label.
    section_counts: BTreeMap<String, usize>,
    /// Labels and counts parsed from the Transaction Summary.
    summary_counts: BTreeMap<String, usize>,
    /// Total number of `replacing` sub-lines seen across all sections.
    replacing_count: usize,
}

impl TableParser {
    fn new() -> Self {
        TableParser {
            state: State::Header,
            updates: Vec::new(),
            section_counts: BTreeMap::new(),
            summary_counts: BTreeMap::new(),
            replacing_count: 0,
        }
    }

    fn feed(&mut self, line: &str) -> anyhow::Result<()> {
        // Take ownership of the state so `Replacing { name }` can be matched by value.
        self.state = match std::mem::replace(&mut self.state, State::Header) {
            State::Header => self.on_header(line)?,
            State::ExpectSection => self.on_expect_section(line)?,
            State::Section { group, upgrading } => self.on_section(line, group, upgrading)?,
            State::Replacing { name } => self.on_replacing(line, &name)?,
            State::SummaryHeader => self.on_summary_header(line)?,
            State::Summary => self.on_summary(line)?,
            State::End => self.on_end(line)?,
        };
        Ok(())
    }

    fn on_header(&self, line: &str) -> anyhow::Result<State> {
        if line.is_empty() {
            Ok(State::Header)
        } else if is_column_header(line) {
            Ok(State::ExpectSection)
        } else if line.trim() == "Nothing to do." {
            Ok(State::End)
        } else {
            bail!("expected the column header 'Package Arch Version Repository Size'");
        }
    }

    fn on_expect_section(&self, line: &str) -> anyhow::Result<State> {
        let name =
            section_header(line).ok_or_else(|| anyhow::anyhow!("expected a section header"))?;
        let group = summary_group(name)
            .ok_or_else(|| anyhow::anyhow!("unknown section header {name:?}"))?;
        Ok(State::Section {
            group,
            upgrading: name == "Upgrading",
        })
    }

    fn on_section(
        &mut self,
        line: &str,
        group: &'static str,
        upgrading: bool,
    ) -> anyhow::Result<State> {
        if line.is_empty() {
            return Ok(State::SummaryHeader);
        }
        if let Some(name) = section_header(line) {
            let group = summary_group(name)
                .ok_or_else(|| anyhow::anyhow!("unknown section header {name:?}"))?;
            return Ok(State::Section {
                group,
                upgrading: name == "Upgrading",
            });
        }
        let row = parse_package_row(line)?;
        *self.section_counts.entry(group.to_string()).or_default() += 1;
        if upgrading {
            self.updates.push(PackageUpdate {
                name: row.name.to_string(),
                arch: row.arch.to_string(),
                new_version: normalize_version(row.version),
                old_version: String::new(),
                old_repo: String::new(),
                new_repo: row.repo.to_string(),
                download_size: row.size,
            });
            Ok(State::Replacing {
                name: row.name.to_string(),
            })
        } else {
            Ok(State::Section { group, upgrading })
        }
    }

    fn on_replacing(&mut self, line: &str, name: &str) -> anyhow::Result<State> {
        let rest = line
            .strip_prefix("   replacing ")
            .ok_or_else(|| anyhow::anyhow!("expected a 'replacing' sub-line for {name:?}"))?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() != 6 {
            bail!("'replacing' line has {} fields, expected 6", fields.len());
        }
        if fields[0] != name {
            bail!(
                "'replacing' references {:?} but expected {name:?}",
                fields[0]
            );
        }
        let update = self
            .updates
            .last_mut()
            .expect("an upgrade package precedes every replacing line");
        update.old_version = normalize_version(fields[2]);
        update.old_repo = fields[3].to_string();
        self.replacing_count += 1;
        Ok(State::Section {
            group: "Upgrading",
            upgrading: true,
        })
    }

    fn on_summary_header(&self, line: &str) -> anyhow::Result<State> {
        if section_header(line) == Some("Transaction Summary") {
            Ok(State::Summary)
        } else {
            bail!("expected 'Transaction Summary:'");
        }
    }

    fn on_summary(&mut self, line: &str) -> anyhow::Result<State> {
        if line.is_empty() {
            return Ok(State::End);
        }
        let (label, count) = parse_summary_count(line)?;
        if self.summary_counts.insert(label.clone(), count).is_some() {
            bail!("duplicate summary label {label:?}");
        }
        Ok(State::Summary)
    }

    fn on_end(&self, line: &str) -> anyhow::Result<State> {
        if line.is_empty() {
            Ok(State::End)
        } else {
            bail!("unexpected content after the transaction summary");
        }
    }

    fn finish(mut self) -> anyhow::Result<Vec<PackageUpdate>> {
        match self.state {
            // No table at all (empty stdout or `Nothing to do.`) → nothing to upgrade.
            State::Header | State::End
                if self.updates.is_empty() && self.summary_counts.is_empty() =>
            {
                return Ok(Vec::new());
            }
            State::Summary | State::End => {}
            other => bail!("dnf output ended unexpectedly in state {other:?}"),
        }

        // Cross-check: the parsed sections must reproduce the Transaction Summary
        // exactly. `replacing` lines map to the summary's `Replacing` bucket.
        let mut expected = self.section_counts.clone();
        if self.replacing_count > 0 {
            expected.insert("Replacing".to_string(), self.replacing_count);
        }
        if expected != self.summary_counts {
            bail!(
                "transaction summary disagrees with the parsed table\n  parsed:  {expected:?}\n  summary: {:?}",
                self.summary_counts
            );
        }

        self.updates.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(self.updates)
    }
}

/// State of the strict line-by-line parser over the `dnf upgrade` transaction
/// table on stdout. Each state names exactly what the *next* line is allowed to
/// be; any line that does not match the active state is a hard error.
#[derive(Debug)]
enum State {
    /// Before anything: expect the column header, `Nothing to do.`, or blanks.
    Header,
    /// Just after the column header: expect the first section header.
    ExpectSection,
    /// Inside a transaction section: expect a package line, a new section
    /// header, or the blank line that introduces the summary. `group` is the
    /// summary bucket the section's packages count toward; `upgrading` is true
    /// only for the `Upgrading:` section (whose package lines carry a
    /// `replacing` sub-line).
    Section {
        group: &'static str,
        upgrading: bool,
    },
    /// Immediately after an upgrade package line: the next line must be its
    /// `replacing` sub-line, naming `name`.
    Replacing { name: String },
    /// After the blank line that ends the sections: expect `Transaction Summary:`.
    SummaryHeader,
    /// Inside the summary: expect ` Label: N package(s)` lines, a trailing
    /// blank, or end of output.
    Summary,
    /// After the trailing blank: only further blank lines are tolerated.
    End,
}

/// One whitespace-delimited transaction-table row (package or `replacing` line).
struct PkgRow<'a> {
    name: &'a str,
    arch: &'a str,
    version: &'a str,
    repo: &'a str,
    size: u64,
}

/// Maps a table section header (without its trailing `:`) to the aggregated
/// label dnf uses for it in the Transaction Summary. `None` means the header is
/// unknown — treated as a hard error so new dnf section types surface loudly
/// rather than being silently skipped.
fn summary_group(section: &str) -> Option<&'static str> {
    Some(match section {
        "Installing" | "Installing dependencies" | "Installing weak dependencies" => "Installing",
        "Upgrading" => "Upgrading",
        "Downgrading" => "Downgrading",
        "Reinstalling" => "Reinstalling",
        "Removing" | "Removing dependent packages" | "Removing unused dependencies" => "Removing",
        _ => return None,
    })
}

/// A column-0, non-empty line ending in `:` is a section/summary header; returns
/// the text without the trailing `:`.
fn section_header(line: &str) -> Option<&str> {
    if line.is_empty() || line.starts_with(' ') {
        return None;
    }
    line.strip_suffix(':')
}

fn is_column_header(line: &str) -> bool {
    line.split_whitespace().collect::<Vec<_>>()
        == ["Package", "Arch", "Version", "Repository", "Size"]
}

/// Parses a single-space-indented, six-field transaction row. Rejects any other
/// indent (e.g. the three-space `replacing` lines) or field count.
fn parse_package_row(line: &str) -> anyhow::Result<PkgRow<'_>> {
    if !line.starts_with(' ') || line.starts_with("  ") {
        bail!("expected a single-space-indented package line");
    }
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() != 6 {
        bail!("package line has {} fields, expected 6", fields.len());
    }
    let size = parse_dnf_size(fields[4], fields[5]).context("parsing package size")?;
    Ok(PkgRow {
        name: fields[0],
        arch: fields[1],
        version: fields[2],
        repo: fields[3],
        size,
    })
}

/// Parses a summary count line such as ` Upgrading:        215 packages` into
/// (label, count). The label is everything before the count, sans its `:`.
fn parse_summary_count(line: &str) -> anyhow::Result<(String, usize)> {
    if !line.starts_with(' ') || line.starts_with("  ") {
        bail!("expected a single-space-indented summary line");
    }
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 3 {
        bail!(
            "summary line has {} fields, expected at least 3",
            fields.len()
        );
    }
    let (unit, count, label_tokens) = (
        fields[fields.len() - 1],
        fields[fields.len() - 2],
        &fields[..fields.len() - 2],
    );
    if unit != "packages" && unit != "package" {
        bail!("summary line must end with 'package(s)', got {unit:?}");
    }
    let count: usize = count
        .parse()
        .with_context(|| format!("invalid summary count {count:?}"))?;
    let label = label_tokens.join(" ");
    let label = label
        .strip_suffix(':')
        .ok_or_else(|| anyhow::anyhow!("summary label {label:?} must end with ':'"))?
        .to_string();
    Ok((label, count))
}

fn normalize_version(v: &str) -> String {
    v.strip_prefix("0:").unwrap_or(v).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compact transaction exercising every line type: column header, a
    /// non-upgrade section (Removing), the Upgrading section with `replacing`
    /// sub-lines, another non-upgrade section (Installing), the blank/summary
    /// boundary, all four summary buckets, and a trailing blank line.
    const SAMPLE: &str = "\
Package    Arch   Version          Repository   Size
Removing:
 oldpkg    x86_64 0:1.0-1.fc44     updates      1.0 MiB
Upgrading:
 bar       noarch 1:2.0-1.fc44     fedora       0.0   B
   replacing bar noarch 1:1.9-1.fc44 fedora     0.0   B
 foo       x86_64 0:2.0-1.fc44     updates      3.3 MiB
   replacing foo x86_64 0:1.0-1.fc44 <unknown>  3.3 MiB
Installing:
 newpkg    x86_64 0:3.0-1.fc44     updates      2.0 MiB

Transaction Summary:
 Installing:   1 package
 Upgrading:    2 packages
 Replacing:    2 packages
 Removing:     1 package
";

    #[test]
    fn parses_sample_transaction() {
        let updates = parse_update_lines(SAMPLE).expect("sample parses");
        // Only Upgrading packages become updates, sorted by name.
        assert_eq!(updates.len(), 2);

        assert_eq!(updates[0].name, "bar");
        assert_eq!(updates[0].arch, "noarch");
        assert_eq!(updates[0].old_version, "1:1.9-1.fc44"); // non-zero epoch preserved
        assert_eq!(updates[0].new_version, "1:2.0-1.fc44");
        assert_eq!(updates[0].old_repo, "fedora");
        assert_eq!(updates[0].new_repo, "fedora");

        assert_eq!(updates[1].name, "foo");
        assert_eq!(updates[1].old_version, "1.0-1.fc44"); // `0:` epoch stripped
        assert_eq!(updates[1].new_version, "2.0-1.fc44");
        assert_eq!(updates[1].old_repo, "<unknown>");
        assert_eq!(updates[1].new_repo, "updates");
        assert_eq!(updates[1].download_size, (3.3 * (1u64 << 20) as f64) as u64);
    }

    #[test]
    fn parses_real_world_capture() {
        // A full 215-upgrade transaction captured from dnf5 5.4.2.1.
        let stdout = include_str!("testdata/dnf_upgrade_stdout.txt");
        let updates = parse_update_lines(stdout).expect("real capture parses");
        assert_eq!(updates.len(), 215);
        assert!(
            updates.windows(2).all(|w| w[0].name <= w[1].name),
            "sorted by name"
        );
        // Spot-check a package with a multi-digit epoch.
        let bind = updates
            .iter()
            .find(|u| u.name == "bind-libs")
            .expect("bind-libs present");
        assert_eq!(bind.old_version, "32:9.18.49-1.fc44");
        assert_eq!(bind.new_version, "32:9.18.50-1.fc44");
    }

    #[test]
    fn empty_output_is_no_updates() {
        assert!(parse_update_lines("").unwrap().is_empty());
    }

    #[test]
    fn nothing_to_do_is_no_updates() {
        assert!(parse_update_lines("Nothing to do.\n").unwrap().is_empty());
    }

    fn err(stdout: &str) -> String {
        format!("{:#}", parse_update_lines(stdout).unwrap_err())
    }

    #[test]
    fn rejects_missing_column_header() {
        assert!(
            err("Removing:\n oldpkg x86_64 0:1.0-1.fc44 updates 1.0 MiB\n")
                .contains("column header")
        );
    }

    #[test]
    fn rejects_unknown_section_header() {
        let s = "Package Arch Version Repository Size\nFrobnicating:\n";
        assert!(err(s).contains("unknown section header"));
    }

    #[test]
    fn rejects_wrong_field_count() {
        let s = "Package Arch Version Repository Size\nRemoving:\n oldpkg x86_64 0:1.0-1.fc44 updates\n";
        assert!(err(s).contains("expected 6"));
    }

    #[test]
    fn rejects_missing_replacing_line() {
        let s = "\
Package Arch Version Repository Size
Upgrading:
 foo x86_64 0:2.0-1.fc44 updates 3.3 MiB
 bar x86_64 0:2.0-1.fc44 updates 3.3 MiB
";
        assert!(err(s).contains("replacing"));
    }

    #[test]
    fn rejects_replacing_name_mismatch() {
        let s = "\
Package Arch Version Repository Size
Upgrading:
 foo x86_64 0:2.0-1.fc44 updates 3.3 MiB
   replacing bar x86_64 0:1.0-1.fc44 updates 3.3 MiB
";
        assert!(err(s).contains("expected \"foo\""));
    }

    #[test]
    fn rejects_truncated_output() {
        let s = "\
Package Arch Version Repository Size
Upgrading:
 foo x86_64 0:2.0-1.fc44 updates 3.3 MiB
   replacing foo x86_64 0:1.0-1.fc44 updates 3.3 MiB
";
        assert!(err(s).contains("ended unexpectedly"));
    }

    #[test]
    fn rejects_summary_count_mismatch() {
        // Summary claims 2 upgrades but only 1 was listed.
        let s = "\
Package Arch Version Repository Size
Upgrading:
 foo x86_64 0:2.0-1.fc44 updates 3.3 MiB
   replacing foo x86_64 0:1.0-1.fc44 updates 3.3 MiB

Transaction Summary:
 Upgrading: 2 packages
 Replacing: 1 package
";
        assert!(err(s).contains("disagrees with the parsed table"));
    }

    #[test]
    fn rejects_orphan_replacing_line() {
        let s = "\
Package Arch Version Repository Size
Upgrading:
   replacing foo x86_64 0:1.0-1.fc44 updates 3.3 MiB
";
        // A replacing line with no preceding package is a single-space/indent mismatch.
        assert!(parse_update_lines(s).is_err());
    }
}
