use crate::{schedule, ssh::Gpu, Snapshot};
use std::sync::Mutex;
use tauri::Manager;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Busy,
    Idle,
    Unknown,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Busy => "忙碌",
            Self::Idle => "空闲",
            Self::Unknown => "离线 / 等待数据",
        }
    }

    fn color(self) -> [u8; 3] {
        match self {
            Self::Busy => [239, 74, 74],
            Self::Idle => [45, 196, 112],
            Self::Unknown => [150, 155, 164],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Dot {
    name: String,
    status: Status,
}

#[derive(Default)]
pub struct State(Mutex<Option<Vec<Dot>>>);

fn status(gpu: &Gpu) -> Status {
    let Some(utilization) = gpu.utilization.filter(|value| value.is_finite()) else {
        return Status::Unknown;
    };
    if utilization >= 10.0 {
        return Status::Busy;
    }
    // Unified RAM usage does not tell us whether an Apple GPU is busy.
    if gpu.unified_memory || gpu.available() {
        return Status::Idle;
    }
    match (gpu.memory_used, gpu.memory_total) {
        (Some(used), Some(total)) if used.is_finite() && total.is_finite() && total > 0.0 => {
            Status::Busy
        }
        _ => Status::Unknown,
    }
}

fn dots(snapshot: &Snapshot, now: u64) -> Vec<Dot> {
    let mut dots = Vec::new();
    for server in &snapshot.servers {
        if !snapshot.monitor(&server.config.id).gpu_enabled {
            continue;
        }
        if server.gpus.is_empty() {
            dots.push(Dot {
                name: format!("{} · GPU 数量待检测", server.config.name),
                status: Status::Unknown,
            });
        }
        for gpu in &server.gpus {
            let updated = server
                .gpu_updated_at
                .get(&gpu.uuid)
                .copied()
                .or(server.updated_at);
            let policy = schedule::resolve_policy(
                &snapshot.global.gpu,
                &snapshot.policies,
                &server.config.id,
                &gpu.uuid,
            );
            let deadline = server.next_poll.get(&gpu.uuid).copied().unwrap_or_else(|| {
                updated
                    .unwrap_or(0)
                    .saturating_add(policy.seconds(gpu.utilization))
            });
            let live = server.status == "online"
                && server.error.is_none()
                && updated.is_some()
                && now <= deadline.saturating_add(30);
            dots.push(Dot {
                name: format!("{} · GPU {} · {}", server.config.name, gpu.index, gpu.name),
                status: if live { status(gpu) } else { Status::Unknown },
            });
        }
    }
    dots
}

fn icon(dots: &[Dot]) -> tauri::image::Image<'static> {
    let count = dots.len().max(1);
    let rows = if count > 8 { 2 } else { 1 };
    let width = (count.div_ceil(rows) * 20).max(36);
    let height = 36;
    let mut pixels = vec![0; width * height * 4];
    for i in 0..count {
        let color = dots
            .get(i)
            .map_or(Status::Unknown, |dot| dot.status)
            .color();
        let cx = if count == 1 {
            width as f64 / 2.0
        } else {
            (i / rows * 20 + 10) as f64
        };
        let cy = if rows == 1 {
            18.0
        } else {
            (i % rows * 18 + 9) as f64
        };
        for y in 0..height {
            for x in (cx as usize).saturating_sub(8)..((cx as usize + 8).min(width)) {
                let distance =
                    ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
                let alpha = ((7.0 - distance).clamp(0.0, 1.0) * 255.0).round() as u8;
                if alpha > 0 {
                    let offset = (y * width + x) * 4;
                    pixels[offset..offset + 3].copy_from_slice(&color);
                    pixels[offset + 3] = alpha;
                }
            }
        }
    }
    tauri::image::Image::new_owned(pixels, width as u32, height as u32)
}

pub fn update(app: &tauri::AppHandle) {
    let dots = dots(
        &app.state::<crate::AppState>().snapshot.lock().unwrap(),
        crate::now(),
    );
    let handle = app.clone();
    // Cache and native tray updates stay on the same thread to avoid lock inversion.
    let _ = app.run_on_main_thread(move || render(&handle, dots));
}

fn render(app: &tauri::AppHandle, dots: Vec<Dot>) {
    let Some(state) = app.try_state::<State>() else {
        return;
    };
    let mut previous = state.0.lock().unwrap();
    if previous.as_ref() == Some(&dots) {
        return;
    }
    let Some(tray) = app.tray_by_id("gpu") else {
        return;
    };
    let mut tooltip = String::from("GPU Monitor · 点击打开面板");
    if dots.is_empty() {
        tooltip.push_str("\n尚未启用 GPU 监控");
    }
    for dot in &dots {
        tooltip.push_str(&format!("\n{}：{}", dot.name, dot.status.label()));
    }
    if tray.set_icon(Some(icon(&dots))).is_ok() && tray.set_tooltip(Some(tooltip)).is_ok() {
        *previous = Some(dots);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> Gpu {
        Gpu {
            index: 0,
            uuid: "gpu".into(),
            name: "GPU".into(),
            utilization: Some(0.0),
            memory_used: Some(0.0),
            memory_total: Some(100.0),
            unified_memory: false,
            temperature: None,
        }
    }

    #[test]
    fn busy_idle_and_unknown_do_not_confuse_shared_memory_with_dedicated_vram() {
        let mut gpu = gpu();
        assert_eq!(status(&gpu), Status::Idle);
        gpu.memory_used = Some(10.0);
        assert_eq!(status(&gpu), Status::Busy);
        gpu.unified_memory = true;
        assert_eq!(status(&gpu), Status::Idle);
        gpu.utilization = Some(10.0);
        assert_eq!(status(&gpu), Status::Busy);
        gpu.utilization = None;
        assert_eq!(status(&gpu), Status::Unknown);
        gpu.utilization = Some(0.0);
        gpu.unified_memory = false;
        gpu.memory_used = None;
        assert_eq!(status(&gpu), Status::Unknown);
    }

    #[test]
    fn offline_and_stale_gpus_keep_their_dots_and_disabled_servers_disappear() {
        let mut snapshot = crate::settings_tests::snapshot();
        for server in &mut snapshot.servers {
            server.status = "online".into();
            server.updated_at = Some(990);
        }
        assert_eq!(
            dots(&snapshot, 1000)
                .iter()
                .filter(|dot| dot.status == Status::Idle)
                .count(),
            4
        );
        snapshot.servers[0].status = "error".into();
        let offline = dots(&snapshot, 1000);
        assert_eq!(offline.len(), 4);
        assert!(offline[..2].iter().all(|dot| dot.status == Status::Unknown));
        assert!(dots(&snapshot, 1030)
            .iter()
            .all(|dot| dot.status == Status::Unknown));
        snapshot.global.monitor.gpu_enabled = false;
        assert!(dots(&snapshot, 1000).is_empty());
    }

    #[test]
    fn empty_and_multi_row_icons_have_valid_rgba_buffers() {
        for count in [0, 1, 8, 9, 32] {
            let dots = vec![
                Dot {
                    name: String::new(),
                    status: Status::Idle
                };
                count
            ];
            let image = icon(&dots);
            assert_eq!(
                image.rgba().len(),
                (image.width() * image.height() * 4) as usize
            );
            assert!(image.rgba().chunks_exact(4).any(|pixel| pixel[3] > 0));
        }
    }
}
