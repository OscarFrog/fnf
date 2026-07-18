use anyhow::{Context, bail};

pub fn parse_dnf_size(number: &str, unit: &str) -> anyhow::Result<u64> {
    let n: f64 = number
        .parse()
        .with_context(|| format!("invalid size number: {number:?}"))?;
    Ok(match unit {
        "GiB" => (n * (1u64 << 30) as f64) as u64,
        "MiB" => (n * (1u64 << 20) as f64) as u64,
        "KiB" => (n * (1u64 << 10) as f64) as u64,
        "B" => n as u64,
        _ => bail!("unknown size unit: {unit:?}"),
    })
}
