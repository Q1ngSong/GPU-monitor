use crate::ssh::Gpu;
use plist::Value;
use std::{process::Stdio, time::Duration};
use tokio::process::Command;

const MIB: f64 = 1024.0 * 1024.0;

async fn read(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(program)
            .args(args)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| "读取 Mac GPU 超时。".to_string())?
    .map_err(|error| format!("无法读取 Mac GPU：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "读取 Mac GPU 失败：{}",
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(300)
                .collect::<String>()
        ));
    }
    Ok(output.stdout)
}

/// Read the Apple driver counters as a regular user; no helper or password is needed.
pub async fn query_gpus(uuids: Option<&[String]>) -> Result<Vec<Gpu>, String> {
    let registry = read(
        "/usr/sbin/ioreg",
        &["-r", "-c", "IOAccelerator", "-d", "1", "-a"],
    )
    .await?;
    let total = read("/usr/sbin/sysctl", &["-n", "hw.memsize"])
        .await
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|bytes| *bytes > 0)
        .map(|bytes| bytes as f64 / MIB);
    parse_gpus(&registry, total, uuids)
}

fn statistic(stats: &plist::Dictionary, key: &str) -> Option<f64> {
    let value = stats.get(key)?;
    value
        .as_unsigned_integer()
        .map(|number| number as f64)
        .or_else(|| value.as_real())
        .filter(|number| number.is_finite() && *number >= 0.0)
}

fn parse_gpus(
    bytes: &[u8],
    total_mib: Option<f64>,
    selected: Option<&[String]>,
) -> Result<Vec<Gpu>, String> {
    let registry = Value::from_reader(std::io::Cursor::new(bytes))
        .map_err(|error| format!("Mac GPU 数据格式异常：{error}"))?;
    let entries = registry.as_array().ok_or("Mac GPU 设备列表格式异常。")?;
    let mut gpus = Vec::new();
    for entry in entries {
        let Some(device) = entry.as_dictionary() else {
            continue;
        };
        let class = device
            .get("IOClass")
            .and_then(Value::as_string)
            .unwrap_or("");
        if !class.starts_with("AGXAccelerator") {
            continue;
        }
        let name = device
            .get("model")
            .and_then(Value::as_string)
            .unwrap_or("Apple GPU");
        let entry_name = device
            .get("IORegistryEntryName")
            .and_then(Value::as_string)
            .unwrap_or(class);
        let stats = device
            .get("PerformanceStatistics")
            .and_then(Value::as_dictionary)
            .ok_or("Apple GPU 驱动未提供性能统计。")?;
        // This is GPU memory currently in use. The denominator is shared physical RAM,
        // not dedicated VRAM or an amount reserved for GPU workloads.
        let used = statistic(stats, "In use system memory")
            .map(|bytes| bytes / MIB)
            .filter(|used| total_mib.is_none_or(|total| *used <= total));
        gpus.push(Gpu {
            index: gpus.len() as u32,
            uuid: format!("APPLE-{entry_name}"),
            name: name.to_string(),
            utilization: statistic(stats, "Device Utilization %").filter(|value| *value <= 100.0),
            memory_used: used,
            memory_total: total_mib,
            unified_memory: true,
            temperature: None,
        });
    }
    if gpus.is_empty() {
        return Err(
            "未检测到 Apple GPU；Mac 本机监控目前支持 Apple 芯片，Intel/AMD GPU 暂不支持。".into(),
        );
    }
    if let Some(selected) = selected {
        if selected.is_empty()
            || selected
                .iter()
                .any(|id| !gpus.iter().any(|gpu| &gpu.uuid == id))
        {
            return Err("部分 GPU 已不可见，请点击立即刷新重新发现设备。".into());
        }
        gpus.retain(|gpu| selected.contains(&gpu.uuid));
    }
    Ok(gpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(stats: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?><plist version="1.0"><array><dict>
            <key>IOClass</key><string>AGXAcceleratorG16G</string>
            <key>IORegistryEntryName</key><string>AGXAcceleratorG16G</string>
            <key>model</key><string>Apple M4</string>
            <key>PerformanceStatistics</key><dict>{stats}</dict>
            </dict></array></plist>"#
        )
        .into_bytes()
    }

    #[test]
    fn apple_counters_use_mib_and_keep_shared_memory_distinct() {
        let bytes = fixture("<key>Device Utilization %</key><integer>56</integer><key>In use system memory</key><integer>1610612736</integer><key>Alloc system memory</key><integer>4294967296</integer>");
        let gpu = parse_gpus(&bytes, Some(16384.0), None).unwrap().remove(0);
        assert_eq!(gpu.name, "Apple M4");
        assert_eq!(gpu.utilization, Some(56.0));
        assert_eq!(gpu.memory_used, Some(1536.0));
        assert_eq!(gpu.memory_total, Some(16384.0));
        assert!(gpu.unified_memory);
        assert!(gpu.temperature.is_none());
        assert_eq!(
            parse_gpus(&bytes, None, Some(&[gpu.uuid.clone()])).unwrap()[0].uuid,
            gpu.uuid
        );
        assert!(parse_gpus(&bytes, None, Some(&["APPLE-missing".into()])).is_err());
        assert!(parse_gpus(&bytes, None, Some(&[])).is_err());
    }

    #[test]
    fn missing_or_invalid_counters_are_unknown_instead_of_zero() {
        for stats in ["", "<key>Device Utilization %</key><integer>101</integer><key>In use system memory</key><integer>-1</integer>"] {
            let gpu = parse_gpus(&fixture(stats), None, None).unwrap().remove(0);
            assert!(gpu.utilization.is_none());
            assert!(gpu.memory_used.is_none());
            assert!(gpu.memory_total.is_none());
        }
        assert!(parse_gpus(b"not a plist", None, None).is_err());
        assert!(parse_gpus(b"<plist version=\"1.0\"><array/></plist>", None, None).is_err());
    }
}
