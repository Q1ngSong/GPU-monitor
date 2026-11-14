use crate::ssh::Gpu;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

pub const WINDOW_SECONDS: u64 = 86400;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub at: u64,
    pub utilization: Option<f64>,
    pub memory_used: Option<f64>,
    pub memory_total: Option<f64>,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
}

fn default_interval() -> u64 {
    15
}

pub type GpuHistory = BTreeMap<String, Vec<Sample>>;
pub type Archive = BTreeMap<String, GpuHistory>;

/// Keep actual samples only; failed polls are explicit breaks in the curve.
pub fn record(
    history: &mut GpuHistory,
    gpus: &[Gpu],
    at: u64,
    online: bool,
    interval_seconds: u64,
    history_seconds: u64,
) {
    for gpu in gpus {
        let samples = history.entry(gpu.uuid.clone()).or_default();
        let sample = Sample {
            at,
            utilization: if online { gpu.utilization } else { None },
            memory_used: if online { gpu.memory_used } else { None },
            memory_total: gpu.memory_total,
            interval_seconds,
        };
        if samples.last().is_some_and(|s| s.at == at) {
            samples.pop();
        }
        samples.push(sample);
        samples.retain(|sample| sample.at >= at.saturating_sub(history_seconds));
    }
    prune(history, at);
}

pub fn prune(history: &mut GpuHistory, now: u64) {
    history.retain(|_, samples| {
        samples
            .retain(|sample| sample.at >= now.saturating_sub(WINDOW_SECONDS) && sample.at <= now);
        samples.sort_by_key(|sample| sample.at);
        !samples.is_empty()
    });
}

pub fn load(path: &Path, now: u64) -> Result<Archive, String> {
    let mut archive: Archive = match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("历史记录无法读取：{e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(format!("历史记录无法读取：{e}")),
    };
    for history in archive.values_mut() {
        prune(history, now);
    }
    Ok(archive)
}

pub fn save(path: &Path, archive: &Archive) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec(archive).map_err(|e| e.to_string())?;
    fs::write(&temporary, bytes).map_err(|e| format!("历史记录保存失败：{e}"))?;
    fs::rename(temporary, path).map_err(|e| format!("历史记录保存失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_hour_records_gaps_and_deduplicates_same_second() {
        let gpu = Gpu {
            index: 0,
            uuid: "GPU-1".into(),
            name: "GPU".into(),
            utilization: Some(95.0),
            memory_used: Some(1024.0),
            memory_total: Some(2048.0),
            temperature: None,
        };
        let mut history = GpuHistory::new();
        record(
            &mut history,
            std::slice::from_ref(&gpu),
            100,
            true,
            15,
            3600,
        );
        record(
            &mut history,
            std::slice::from_ref(&gpu),
            200,
            true,
            15,
            3600,
        );
        record(
            &mut history,
            std::slice::from_ref(&gpu),
            200,
            false,
            15,
            3600,
        );
        assert_eq!(history["GPU-1"].len(), 2);
        assert!(history["GPU-1"][1].utilization.is_none());
        record(&mut history, &[gpu], 3801, true, 15, 3600);
        assert_eq!(history["GPU-1"].len(), 1);
        assert_eq!(history["GPU-1"][0].at, 3801);
    }
}
