use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub mode: String,
    pub fast_seconds: u64,
    pub normal_seconds: u64,
    pub slow_seconds: u64,
    pub fixed_seconds: u64,
    #[serde(default = "default_history")]
    pub history_minutes: u64,
}

pub fn default_history() -> u64 {
    60
}
pub const HISTORY_MINUTES: [u64; 6] = [15, 30, 60, 180, 360, 1440];

impl Default for Policy {
    fn default() -> Self {
        Self {
            mode: "adaptive".into(),
            fast_seconds: 5,
            normal_seconds: 15,
            slow_seconds: 30,
            fixed_seconds: 15,
            history_minutes: 60,
        }
    }
}

impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if !["adaptive", "fixed"].contains(&self.mode.as_str()) {
            return Err("请选择自动调节或固定间隔。".into());
        }
        if [
            self.fast_seconds,
            self.normal_seconds,
            self.slow_seconds,
            self.fixed_seconds,
        ]
        .iter()
        .any(|s| !(2..=3600).contains(s))
        {
            return Err("刷新间隔应在 2–3600 秒之间。".into());
        }
        if self.fast_seconds > self.normal_seconds || self.normal_seconds > self.slow_seconds {
            return Err("自动调节时，低利用率的间隔应不大于中、高利用率的间隔。".into());
        }
        if !HISTORY_MINUTES.contains(&self.history_minutes) {
            return Err("请选择支持的历史长度。".into());
        }
        Ok(())
    }

    /// Scheduling depends on utilization only, independently of the low-load badge.
    pub fn seconds(&self, utilization: Option<f64>) -> u64 {
        if self.mode == "fixed" {
            return self.fixed_seconds;
        }
        match utilization {
            Some(value) if value < 10.0 => self.fast_seconds,
            Some(value) if value < 60.0 => self.normal_seconds,
            _ => self.slow_seconds,
        }
    }
}

pub fn key(server_id: &str, gpu_uuid: &str) -> String {
    format!("{server_id}/{gpu_uuid}")
}

pub fn resolve_policy<'a>(
    global: &'a Policy,
    overrides: &'a BTreeMap<String, Policy>,
    server_id: &str,
    gpu_uuid: &str,
) -> &'a Policy {
    overrides.get(&key(server_id, gpu_uuid)).unwrap_or(global)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub cpu_enabled: bool,
    pub gpu_enabled: bool,
    pub cpu_seconds: u64,
    pub cpu_history_minutes: u64,
}
impl Default for Monitor {
    fn default() -> Self {
        Self {
            cpu_enabled: false,
            gpu_enabled: true,
            cpu_seconds: 15,
            cpu_history_minutes: 60,
        }
    }
}
impl Monitor {
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=3600).contains(&self.cpu_seconds)
            || !HISTORY_MINUTES.contains(&self.cpu_history_minutes)
        {
            return Err("CPU 刷新间隔应为 2–3600 秒，历史长度请选择列表中的值。".into());
        }
        Ok(())
    }
}

/// Each settings panel overrides only its own fields; old complete entries still deserialize.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MonitorOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_history_minutes: Option<u64>,
}

impl MonitorOverride {
    pub fn resolve(&self, global: &Monitor) -> Monitor {
        Monitor {
            cpu_enabled: self.cpu_enabled.unwrap_or(global.cpu_enabled),
            gpu_enabled: self.gpu_enabled.unwrap_or(global.gpu_enabled),
            cpu_seconds: self.cpu_seconds.unwrap_or(global.cpu_seconds),
            cpu_history_minutes: self
                .cpu_history_minutes
                .unwrap_or(global.cpu_history_minutes),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.resolve(&Monitor::default()).validate()
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

impl From<Monitor> for MonitorOverride {
    fn from(monitor: Monitor) -> Self {
        Self {
            cpu_enabled: Some(monitor.cpu_enabled),
            gpu_enabled: Some(monitor.gpu_enabled),
            cpu_seconds: Some(monitor.cpu_seconds),
            cpu_history_minutes: Some(monitor.cpu_history_minutes),
        }
    }
}

pub fn resolve_monitor(
    global: &Monitor,
    overrides: &BTreeMap<String, MonitorOverride>,
    server_id: &str,
) -> Monitor {
    overrides
        .get(server_id)
        .map_or_else(|| global.clone(), |own| own.resolve(global))
}

/// Defaults are resolved at use time so individual overrides never get overwritten.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Global {
    pub gpu: Policy,
    pub monitor: Monitor,
}

impl Global {
    pub fn validate(&self) -> Result<(), String> {
        self.gpu.validate()?;
        self.monitor.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adaptive_cards_have_independent_deadlines() {
        let policy = Policy::default();
        let busy_deadline = 100 + policy.seconds(Some(98.0));
        let idle_deadline = 100 + policy.seconds(Some(2.0));
        assert_eq!((busy_deadline, idle_deadline), (130, 105));
        assert!(idle_deadline <= 105 && busy_deadline > 105);
        assert_eq!(policy.seconds(Some(10.0)), 15);
        assert_eq!(policy.seconds(Some(60.0)), 30);
        assert_eq!(policy.seconds(None), 30);
        let fixed = Policy {
            mode: "fixed".into(),
            fixed_seconds: 9,
            ..policy
        };
        assert_eq!(fixed.seconds(Some(99.0)), 9);
        assert_eq!(fixed.seconds(Some(0.0)), 9);
    }
    #[test]
    fn invalid_and_reversed_intervals_are_rejected() {
        assert!(Policy {
            fast_seconds: 1,
            ..Policy::default()
        }
        .validate()
        .is_err());
        assert!(Policy {
            fast_seconds: 20,
            ..Policy::default()
        }
        .validate()
        .is_err());
        assert!(Policy {
            fixed_seconds: 3601,
            ..Policy::default()
        }
        .validate()
        .is_err());
    }
}
