use crate::ssh;
use serde::Serialize;
use std::path::Path;

const QUERY: &str = "LC_ALL=C sh -c 'head -n 1 /proc/stat; sleep 0.3; head -n 1 /proc/stat; getconf _NPROCESSORS_ONLN; cat /proc/meminfo'";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cpu {
    pub logical_cpus: u32,
    pub utilization: Option<f64>,
    pub memory_used: f64,
    pub memory_total: f64,
    pub updated_at: u64,
}

pub async fn query(connection: &ssh::Connection, home: &Path) -> Result<Cpu, String> {
    let mut command = ssh::connection_command(connection, home, false)?;
    command.arg(QUERY);
    parse(&ssh::output(command, None, 25).await?, crate::now())
}

fn ticks(line: &str) -> Result<(u64, u64), String> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some("cpu") {
        return Err("CPU 数据不可用；当前支持 Linux /proc。".into());
    }
    let values: Vec<u64> = fields
        .take(8)
        .map(|s| s.parse().map_err(|_| "CPU 计数器格式错误。".to_string()))
        .collect::<Result<_, _>>()?;
    if values.len() < 4 {
        return Err("CPU 计数器不完整。".into());
    }
    let idle = values[3].saturating_add(values.get(4).copied().unwrap_or(0));
    let total = values
        .iter()
        .try_fold(0u64, |a, b| a.checked_add(*b))
        .ok_or("CPU 计数器超出范围。")?;
    Ok((total, idle))
}

pub fn parse(text: &str, at: u64) -> Result<Cpu, String> {
    let mut lines = text.lines();
    let (before_total, before_idle) = ticks(lines.next().ok_or("缺少 CPU 数据。")?)?;
    let (after_total, after_idle) = ticks(lines.next().ok_or("缺少 CPU 数据。")?)?;
    let logical_cpus = lines
        .next()
        .ok_or("缺少 CPU 核心数。")?
        .trim()
        .parse()
        .map_err(|_| "CPU 核心数无效。")?;
    let mut memory_total = None;
    let mut memory_available = None;
    for line in lines {
        let mut fields = line.split_whitespace();
        let key = fields.next();
        if matches!(key, Some("MemTotal:" | "MemAvailable:")) {
            let value: f64 = fields
                .next()
                .ok_or("内存数据不完整。")?
                .parse()
                .map_err(|_| "内存数据无效。")?;
            if !value.is_finite() || value < 0.0 {
                return Err("内存数据超出范围。".into());
            }
            if key == Some("MemTotal:") {
                memory_total = Some(value / 1024.0);
            } else {
                memory_available = Some(value / 1024.0);
            }
        }
    }
    let memory_total = memory_total.ok_or("未找到系统内存总量。")?;
    let available = memory_available.ok_or("未找到 MemAvailable；需要支持该字段的 Linux 内核。")?;
    if logical_cpus == 0 || memory_total <= 0.0 || available > memory_total {
        return Err("CPU 或内存数据超出范围。".into());
    }
    let delta_total = after_total.checked_sub(before_total);
    let delta_idle = after_idle.checked_sub(before_idle);
    let utilization = match (delta_total, delta_idle) {
        (Some(total), Some(idle)) if total > 0 && idle <= total => {
            Some(((total - idle) as f64 / total as f64 * 1000.0).round() / 10.0)
        }
        _ => None,
    };
    Ok(Cpu {
        logical_cpus,
        utilization,
        memory_used: memory_total - available,
        memory_total,
        updated_at: at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_delta_excludes_guest_double_count_and_uses_available_memory() {
        let c = parse("cpu 100 0 50 800 50 0 0 0 20 0\ncpu 120 0 60 860 60 0 0 0 25 0\n8\nMemTotal: 8388608 kB\nMemAvailable: 2097152 kB\n", 1).unwrap();
        assert_eq!(c.utilization, Some(30.0));
        assert_eq!(c.memory_used, 6144.0);
        assert_eq!(c.memory_total, 8192.0);
        assert_eq!(c.logical_cpus, 8);
        assert!(parse("not linux", 1).is_err());
    }
}
