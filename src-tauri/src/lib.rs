pub mod askpass;
pub mod cli;
mod cpu;
mod history;
mod local;
mod overlay;
mod schedule;
mod ssh;
#[cfg(any(target_os = "macos", test))]
mod tray_status;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Source {
    #[default]
    Ssh,
    Local,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerConfig {
    id: String,
    name: String,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    ssh_command: String,
}

impl ServerConfig {
    fn monitor(
        &self,
        global: &schedule::Monitor,
        overrides: &BTreeMap<String, schedule::MonitorOverride>,
    ) -> schedule::Monitor {
        let mut monitor = schedule::resolve_monitor(global, overrides, &self.id);
        if self.source == Source::Local {
            monitor.cpu_enabled = false;
        }
        monitor
    }

    async fn query_gpus(
        &self,
        home: &Path,
        uuids: Option<&[String]>,
    ) -> Result<Vec<ssh::Gpu>, String> {
        match self.source {
            Source::Local => local::query_gpus(uuids).await,
            Source::Ssh => {
                ssh::query_selected(&ssh::parse_connection(&self.ssh_command)?, home, uuids).await
            }
        }
    }

    async fn query_cpu(&self, home: &Path) -> Result<cpu::Cpu, String> {
        match self.source {
            Source::Local => Err("本机目前仅支持 GPU 监控，暂不支持本机 CPU。".into()),
            Source::Ssh => cpu::query(&ssh::parse_connection(&self.ssh_command)?, home).await,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    servers: Vec<ServerConfig>,
    interval: u64,
    #[serde(default)]
    policies: BTreeMap<String, schedule::Policy>,
    #[serde(default)]
    monitors: BTreeMap<String, schedule::MonitorOverride>,
    #[serde(default)]
    global: schedule::Global,
    #[serde(default)]
    overlay: overlay::Settings,
    #[serde(default)]
    overlay_layout: Option<overlay::Layout>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            servers: vec![],
            interval: 15,
            policies: BTreeMap::new(),
            monitors: BTreeMap::new(),
            global: schedule::Global::default(),
            overlay: overlay::Settings::default(),
            overlay_layout: None,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerView {
    #[serde(flatten)]
    config: ServerConfig,
    target: String,
    gpus: Vec<ssh::Gpu>,
    status: String,
    error: Option<String>,
    updated_at: Option<u64>,
    history: history::GpuHistory,
    gpu_updated_at: BTreeMap<String, u64>,
    next_poll: BTreeMap<String, u64>,
    #[serde(skip)]
    in_flight: bool,
    #[serde(skip)]
    full_refresh: bool,
    #[serde(skip)]
    retry_at: u64,
    cpu: Option<cpu::Cpu>,
    cpu_error: Option<String>,
    cpu_next_poll: u64,
    #[serde(skip)]
    cpu_in_flight: bool,
}
impl ServerView {
    fn new(config: ServerConfig) -> Result<Self, String> {
        let target = match config.source {
            Source::Local => "本机".into(),
            Source::Ssh => ssh::parse_connection(&config.ssh_command)?.target,
        };
        Ok(Self {
            config,
            target,
            gpus: vec![],
            status: "connecting".into(),
            error: None,
            updated_at: None,
            history: history::GpuHistory::new(),
            gpu_updated_at: BTreeMap::new(),
            next_poll: BTreeMap::new(),
            in_flight: false,
            full_refresh: true,
            retry_at: 0,
            cpu: None,
            cpu_error: None,
            cpu_next_poll: 0,
            cpu_in_flight: false,
        })
    }
}

#[derive(Clone, Serialize)]
struct Snapshot {
    servers: Vec<ServerView>,
    interval: u64,
    refreshing: bool,
    #[serde(rename = "historyWarning")]
    history_warning: Option<String>,
    policies: BTreeMap<String, schedule::Policy>,
    monitors: BTreeMap<String, schedule::MonitorOverride>,
    global: schedule::Global,
    overlay: overlay::Settings,
    #[serde(rename = "overlayEditing")]
    overlay_editing: bool,
    #[serde(skip)]
    overlay_layout: Option<overlay::Layout>,
}

impl Snapshot {
    fn monitor(&self, server_id: &str) -> schedule::Monitor {
        self.servers
            .iter()
            .find(|s| s.config.id == server_id)
            .map(|s| s.config.monitor(&self.global.monitor, &self.monitors))
            .unwrap_or_else(|| {
                schedule::resolve_monitor(&self.global.monitor, &self.monitors, server_id)
            })
    }

    fn reset_gpu(&mut self, server_id: &str, gpu_uuid: &str, at: u64) -> Result<(), String> {
        let server = self
            .servers
            .iter_mut()
            .find(|s| s.config.id == server_id)
            .ok_or("服务器已移除。")?;
        self.policies.remove(&schedule::key(server_id, gpu_uuid));
        server.next_poll.insert(gpu_uuid.into(), at);
        server.retry_at = 0;
        Ok(())
    }

    fn reset_server_monitor(&mut self, server_id: &str, at: u64) -> Result<(), String> {
        self.replace_monitor_override(server_id, schedule::MonitorOverride::default(), at)
    }

    fn replace_monitor_override(
        &mut self,
        server_id: &str,
        own: schedule::MonitorOverride,
        at: u64,
    ) -> Result<(), String> {
        own.validate()?;
        let before = self.monitor(server_id);
        let mut after = own.resolve(&self.global.monitor);
        let server = self
            .servers
            .iter_mut()
            .find(|s| s.config.id == server_id)
            .ok_or("服务器已移除。")?;
        if server.config.source == Source::Local {
            after.cpu_enabled = false;
        }
        reschedule_monitor(server, &before, &after, at);
        if own.is_empty() {
            self.monitors.remove(server_id);
        } else {
            self.monitors.insert(server_id.into(), own);
        }
        Ok(())
    }

    fn set_monitor_switches(
        &mut self,
        server_id: &str,
        gpu_enabled: bool,
        cpu_enabled: bool,
        at: u64,
    ) -> Result<(), String> {
        let mut own = self.monitors.get(server_id).cloned().unwrap_or_default();
        own.gpu_enabled = Some(gpu_enabled);
        own.cpu_enabled = Some(cpu_enabled);
        self.replace_monitor_override(server_id, own, at)
    }

    fn set_cpu_settings(
        &mut self,
        server_id: &str,
        cpu_seconds: u64,
        cpu_history_minutes: u64,
        at: u64,
    ) -> Result<(), String> {
        let mut own = self.monitors.get(server_id).cloned().unwrap_or_default();
        own.cpu_seconds = Some(cpu_seconds);
        own.cpu_history_minutes = Some(cpu_history_minutes);
        self.replace_monitor_override(server_id, own, at)
    }

    fn reset_monitor_section(
        &mut self,
        server_id: &str,
        section: &str,
        at: u64,
    ) -> Result<(), String> {
        let mut own = self.monitors.get(server_id).cloned().unwrap_or_default();
        match section {
            "switches" => {
                own.gpu_enabled = None;
                own.cpu_enabled = None;
            }
            "cpu" => {
                own.cpu_seconds = None;
                own.cpu_history_minutes = None;
            }
            _ => return Err("未知的设置面板。".into()),
        }
        self.replace_monitor_override(server_id, own, at)
    }

    /// Recalculate only inherited schedules; separately configured cards keep their deadlines.
    fn apply_global(&mut self, settings: schedule::Global, at: u64) {
        let gpu_changed = self.global.gpu != settings.gpu;
        let previous_monitor = self.global.monitor.clone();
        self.global = settings;
        for server in &mut self.servers {
            if gpu_changed {
                for gpu in &server.gpus {
                    if !self
                        .policies
                        .contains_key(&schedule::key(&server.config.id, &gpu.uuid))
                    {
                        server.next_poll.insert(gpu.uuid.clone(), at);
                        server.retry_at = 0;
                    }
                }
            }
            let before = server.config.monitor(&previous_monitor, &self.monitors);
            let after = server.config.monitor(&self.global.monitor, &self.monitors);
            reschedule_monitor(server, &before, &after, at);
        }
    }
}

fn reschedule_monitor(
    server: &mut ServerView,
    before: &schedule::Monitor,
    after: &schedule::Monitor,
    at: u64,
) {
    if before.cpu_enabled != after.cpu_enabled
        || before.cpu_seconds != after.cpu_seconds
        || before.cpu_history_minutes != after.cpu_history_minutes
    {
        server.cpu_next_poll = at;
    }
    if !before.gpu_enabled && after.gpu_enabled {
        server.full_refresh = true;
        server.retry_at = 0;
    }
}

struct AppState {
    config_path: PathBuf,
    history_path: PathBuf,
    history_io: Mutex<()>,
    home: PathBuf,
    snapshot: Mutex<Snapshot>,
    refresh: Notify,
    setup: tokio::sync::Mutex<()>,
    pending: Mutex<Option<askpass::Pending>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn save_history(state: &AppState) {
    let _guard = state.history_io.lock().unwrap();
    let archive = state
        .snapshot
        .lock()
        .unwrap()
        .servers
        .iter()
        .map(|s| (s.config.id.clone(), s.history.clone()))
        .collect();
    if let Err(error) = history::save(&state.history_path, &archive) {
        state.snapshot.lock().unwrap().history_warning = Some(error);
    }
}

/// Write a complete replacement before touching the saved configuration.
fn persist(state: &AppState, snapshot: &Snapshot) -> Result<(), String> {
    let config = Config {
        servers: snapshot.servers.iter().map(|s| s.config.clone()).collect(),
        interval: snapshot.interval,
        policies: snapshot.policies.clone(),
        monitors: snapshot.monitors.clone(),
        global: snapshot.global.clone(),
        overlay: snapshot.overlay.clone(),
        overlay_layout: snapshot.overlay_layout.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    let temporary = state.config_path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|e| format!("保存设置失败：{e}"))?;
    fs::rename(temporary, &state.config_path).map_err(|e| format!("保存设置失败：{e}"))
}

fn publish(app: &tauri::AppHandle) {
    let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
    #[cfg(target_os = "macos")]
    tray_status::update(app);
    #[cfg(not(target_os = "macos"))]
    {
        let online: Vec<_> = snapshot
            .servers
            .iter()
            .filter(|s| {
                let monitor = snapshot.monitor(&s.config.id);
                (monitor.gpu_enabled && s.status == "online")
                    || (monitor.cpu_enabled && s.cpu.is_some() && s.cpu_error.is_none())
            })
            .collect();
        let gpus: Vec<_> = online
            .iter()
            .filter(|s| s.status == "online" && snapshot.monitor(&s.config.id).gpu_enabled)
            .flat_map(|s| &s.gpus)
            .collect();
        let total = gpus.len();
        let available = gpus.into_iter().filter(|g| g.available()).count();
        if let Some(tray) = app.tray_by_id("gpu") {
            let _ = tray.set_tooltip(Some(format!(
                "GPU Monitor · {available}/{total} 张低负载 · {}/{} 台在线",
                online.len(),
                snapshot.servers.len()
            )));
        }
    }
    let _ = app.emit("snapshot", snapshot);
}

#[tauri::command]
fn get_snapshot(state: tauri::State<AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn set_overlay_settings(app: tauri::AppHandle, settings: overlay::Settings) -> Result<(), String> {
    settings.validate()?;
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    if settings
        .server_id
        .as_ref()
        .is_some_and(|id| !guard.servers.iter().any(|server| &server.config.id == id))
    {
        return Err("所选服务器已移除，请选择全部服务器。".into());
    }
    let mut updated = guard.clone();
    updated.overlay = settings.clone();
    if !settings.enabled {
        updated.overlay_editing = false;
    }
    let editing = updated.overlay_editing;
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    overlay::apply(&app, &settings, editing)?;
    publish(&app);
    Ok(())
}

#[tauri::command]
fn begin_overlay_edit(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.overlay.enabled = true;
    updated.overlay_editing = true;
    let settings = updated.overlay.clone();
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    if let Some(window) = app.get_webview_window("overlay") {
        let layout = overlay::capture(&app)?;
        overlay::place(&window, Some(&layout))?;
    }
    overlay::apply(&app, &settings, true)?;
    publish(&app);
    Ok(())
}

#[tauri::command]
fn end_overlay_edit(app: tauri::AppHandle) -> Result<(), String> {
    let layout = overlay::capture(&app)?;
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.overlay_editing = false;
    updated.overlay_layout = Some(layout);
    let settings = updated.overlay.clone();
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    overlay::apply(&app, &settings, false)?;
    publish(&app);
    Ok(())
}

#[tauri::command]
fn open_main_window(app: tauri::AppHandle) {
    show_window(&app);
}

#[tauri::command]
fn add_connection(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    username: String,
) -> Result<String, String> {
    let command = ssh::connection_from_fields(&host, port, &username)?;
    add_server(app, host.trim().to_string(), command)
}

#[tauri::command]
fn add_server(app: tauri::AppHandle, name: String, ssh_command: String) -> Result<String, String> {
    let state = app.state::<AppState>();
    let connection = ssh::parse_connection(ssh_command.trim())?;
    let name = if name.trim().is_empty() {
        connection.target.clone()
    } else {
        name.trim().to_string()
    };
    if name.chars().count() > 80 {
        return Err("名称请控制在 80 个字以内。".into());
    }
    let mut guard = state.snapshot.lock().unwrap();
    if guard.servers.len() >= 32 {
        return Err("最多支持 32 台服务器。".into());
    }
    if let Some(existing) = guard
        .servers
        .iter()
        .find(|s| ssh::parse_connection(&s.config.ssh_command).ok().as_ref() == Some(&connection))
    {
        return Ok(existing.config.id.clone());
    }
    let id = Uuid::new_v4().to_string();
    let mut updated = guard.clone();
    updated.servers.push(ServerView::new(ServerConfig {
        id: id.clone(),
        name,
        source: Source::Ssh,
        ssh_command: ssh_command.trim().into(),
    })?);
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(id)
}

#[tauri::command]
fn add_local(app: tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    if let Some(existing) = guard
        .servers
        .iter()
        .find(|s| s.config.source == Source::Local)
    {
        return Ok(existing.config.id.clone());
    }
    if guard.servers.len() >= 32 {
        return Err("最多支持 32 台设备。".into());
    }
    let id = Uuid::new_v4().to_string();
    let mut updated = guard.clone();
    updated.servers.push(ServerView::new(ServerConfig {
        id: id.clone(),
        name: "本机".into(),
        source: Source::Local,
        ssh_command: String::new(),
    })?);
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(id)
}

#[tauri::command]
fn rename_server(app: tauri::AppHandle, id: String, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("名称请填写 1–80 个字，不能包含换行或控制字符。".into());
    }
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    let server = updated
        .servers
        .iter_mut()
        .find(|server| server.config.id == id)
        .ok_or("设备已移除，请重新选择。")?;
    server.config.name = name.to_string();
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    Ok(())
}

#[tauri::command]
fn remove_server(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.servers.retain(|s| s.config.id != id);
    updated
        .policies
        .retain(|key, _| !key.starts_with(&format!("{id}/")));
    updated.monitors.remove(&id);
    if updated.overlay.server_id.as_deref() == Some(&id) {
        updated.overlay.server_id = None;
    }
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    save_history(&state);
    publish(&app);
    Ok(())
}

#[tauri::command]
fn set_global_settings(app: tauri::AppHandle, settings: schedule::Global) -> Result<(), String> {
    settings.validate()?;
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.apply_global(settings, now());
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn reset_refresh_policy(
    app: tauri::AppHandle,
    server_id: String,
    gpu_uuid: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.reset_gpu(&server_id, &gpu_uuid, now())?;
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn reset_monitor(app: tauri::AppHandle, server_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.reset_server_monitor(&server_id, now())?;
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn set_refresh_policy(
    app: tauri::AppHandle,
    server_id: String,
    gpu_uuid: String,
    policy: schedule::Policy,
) -> Result<(), String> {
    policy.validate()?;
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    let server = updated
        .servers
        .iter_mut()
        .find(|s| s.config.id == server_id && s.gpus.iter().any(|g| g.uuid == gpu_uuid))
        .ok_or("没有找到这张 GPU，请重新刷新。")?;
    server.next_poll.insert(gpu_uuid.clone(), now());
    server.retry_at = 0;
    updated
        .policies
        .insert(schedule::key(&server_id, &gpu_uuid), policy);
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn set_monitor(
    app: tauri::AppHandle,
    server_id: String,
    monitor: schedule::Monitor,
) -> Result<(), String> {
    monitor.validate()?;
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    updated.replace_monitor_override(&server_id, monitor.into(), now())?;
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(&app);
    state.refresh.notify_one();
    Ok(())
}

fn update_monitor_settings(
    app: &tauri::AppHandle,
    update: impl FnOnce(&mut Snapshot) -> Result<(), String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut guard = state.snapshot.lock().unwrap();
    let mut updated = guard.clone();
    update(&mut updated)?;
    persist(&state, &updated)?;
    *guard = updated;
    drop(guard);
    publish(app);
    state.refresh.notify_one();
    Ok(())
}

#[tauri::command]
fn set_monitor_switches(
    app: tauri::AppHandle,
    server_id: String,
    gpu_enabled: bool,
    cpu_enabled: bool,
) -> Result<(), String> {
    update_monitor_settings(&app, |snapshot| {
        snapshot.set_monitor_switches(&server_id, gpu_enabled, cpu_enabled, now())
    })
}

#[tauri::command]
fn set_cpu_settings(
    app: tauri::AppHandle,
    server_id: String,
    cpu_seconds: u64,
    cpu_history_minutes: u64,
) -> Result<(), String> {
    update_monitor_settings(&app, |snapshot| {
        snapshot.set_cpu_settings(&server_id, cpu_seconds, cpu_history_minutes, now())
    })
}

#[tauri::command]
fn reset_monitor_section(
    app: tauri::AppHandle,
    server_id: String,
    section: String,
) -> Result<(), String> {
    update_monitor_settings(&app, |snapshot| {
        snapshot.reset_monitor_section(&server_id, &section, now())
    })
}

#[tauri::command]
fn refresh_now(state: tauri::State<AppState>) {
    request_refresh(&state);
}

fn request_refresh(state: &AppState) {
    for server in &mut state.snapshot.lock().unwrap().servers {
        server.full_refresh = true;
        server.retry_at = 0;
        server.cpu_next_poll = 0;
    }
    state.refresh.notify_one();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyInfo {
    path: String,
    exists: bool,
    public_exists: bool,
}

#[tauri::command]
fn inspect_key(state: tauri::State<AppState>, ssh_command: String) -> Result<KeyInfo, String> {
    let connection = ssh::parse_connection(&ssh_command)?;
    let path = ssh::identity_path(&connection, &state.home);
    Ok(KeyInfo {
        exists: path.is_file(),
        public_exists: PathBuf::from(format!("{}.pub", path.display())).is_file(),
        path: path.to_string_lossy().into(),
    })
}

#[tauri::command]
fn get_auth_prompt(state: tauri::State<AppState>) -> Option<askpass::Prompt> {
    state
        .pending
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.prompt.clone())
}

#[tauri::command]
fn answer_prompt(
    state: tauri::State<AppState>,
    id: String,
    value: Option<String>,
) -> Result<(), String> {
    let mut pending = state.pending.lock().unwrap();
    if pending.as_ref().map(|p| &p.prompt.id) != Some(&id) {
        return Err("该认证请求已过期，请重新连接。".into());
    }
    if value
        .as_ref()
        .is_some_and(|s| s.len() > 16384 || s.contains(['\n', '\r', '\0']))
    {
        return Err("输入包含无效字符。".into());
    }
    if let Some(request) = pending.take() {
        let _ = request.answer.send(value);
    }
    Ok(())
}

#[tauri::command]
async fn setup_server(
    app: tauri::AppHandle,
    id: String,
    password: Option<String>,
) -> Result<(), String> {
    if password
        .as_ref()
        .is_some_and(|s| s.len() > 16384 || s.contains(['\r', '\n', '\0']))
    {
        return Err("密码包含不支持的字符。".into());
    }
    let state = app.state::<AppState>();
    let _lock = state
        .setup
        .try_lock()
        .map_err(|_| "另一台服务器正在接入，请先完成或取消。")?;
    let config = state
        .snapshot
        .lock()
        .unwrap()
        .servers
        .iter()
        .find(|s| s.config.id == id)
        .map(|s| s.config.clone())
        .ok_or("服务器已移除。")?;
    if config.source == Source::Local {
        return Err("本机无需 SSH 接入，请使用重新检测。".into());
    }
    let connection = ssh::parse_connection(&config.ssh_command)?;
    let identity = ssh::identity_path(&connection, &state.home);
    let password = password.filter(|s| !s.is_empty());
    let password_target = if password.is_some() {
        ssh::direct_password_target(&connection, &state.home).await
    } else {
        None
    };
    let bridge = askpass::Bridge::new(
        app.clone(),
        password.filter(|_| password_target.is_some()),
        password_target.as_ref().map(ssh::PasswordTarget::prompt),
    )
    .await?;
    if !identity.is_file() {
        if connection.identity.is_some() {
            return Err("指定的私钥文件不存在。".into());
        }
        if PathBuf::from(format!("{}.pub", identity.display())).exists() {
            return Err("发现同名公钥但没有私钥；为避免覆盖，请先处理该文件。".into());
        }
        let parent = identity.parent().ok_or("密钥路径无效。")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let mut cmd = ssh::command("ssh-keygen");
        cmd.args(["-t", "ed25519", "-N", "", "-C", "gpu-monitor", "-f"])
            .arg(&identity);
        ssh::output(cmd, None, 15).await?;
    }
    let public_path = PathBuf::from(format!("{}.pub", identity.display()));
    if !public_path.is_file() {
        let mut cmd = ssh::command("ssh-keygen");
        cmd.arg("-y").arg("-f").arg(&identity);
        bridge.configure(&mut cmd)?;
        let public = ssh::output(cmd, None, 180).await?;
        fs::write(&public_path, public).map_err(|e| e.to_string())?;
    }
    let public = fs::read(&public_path).map_err(|e| e.to_string())?;
    if public.len() > 32768 {
        return Err("公钥文件异常。".into());
    }
    let mut cmd = ssh::setup_command(&connection, &state.home, password_target.as_ref())?;
    cmd.arg(ssh::INSTALL_KEY);
    bridge.configure(&mut cmd)?;
    ssh::output(cmd, Some(public), 240).await?;
    drop(bridge);
    // Verify authentication independently of the chosen CPU/GPU collectors.
    let mut verify = ssh::verification_command(&connection, &state.home, password_target.as_ref())?;
    verify.arg("printf GPU_MONITOR_SSH_OK");
    ssh::output(verify, None, 25).await.map_err(|e| {
        format!("公钥已写入，但免密连接验证失败。带口令的密钥需要先在 ssh-agent 中解锁。\n{e}")
    })?;
    if let Some(server) = state
        .snapshot
        .lock()
        .unwrap()
        .servers
        .iter_mut()
        .find(|s| s.config.id == id)
    {
        server.full_refresh = true;
        server.retry_at = 0;
        server.cpu_next_poll = 0;
    }
    publish(&app);
    state.refresh.notify_one();
    Ok(())
}

async fn poll(app: tauri::AppHandle) {
    loop {
        let state = app.state::<AppState>();
        let (jobs, cpu_jobs) = {
            let mut snapshot = state.snapshot.lock().unwrap();
            let monitors = snapshot.monitors.clone();
            let default_monitor = snapshot.global.monitor.clone();
            let mut jobs = Vec::new();
            let mut cpu_jobs = Vec::new();
            for server in &mut snapshot.servers {
                let monitor = server.config.monitor(&default_monitor, &monitors);
                if monitor.cpu_enabled && !server.cpu_in_flight && server.cpu_next_poll <= now() {
                    server.cpu_in_flight = true;
                    cpu_jobs.push(server.config.clone());
                }
                if !monitor.gpu_enabled {
                    continue;
                }
                if server.in_flight || server.retry_at > now() {
                    continue;
                }
                let selection = if server.full_refresh || server.gpus.is_empty() {
                    None
                } else {
                    let due: Vec<_> = server
                        .gpus
                        .iter()
                        .filter(|g| server.next_poll.get(&g.uuid).copied().unwrap_or(0) <= now())
                        .map(|g| g.uuid.clone())
                        .collect();
                    if due.is_empty() {
                        continue;
                    }
                    Some(due)
                };
                server.in_flight = true;
                server.full_refresh = false;
                jobs.push((server.config.clone(), selection));
            }
            snapshot.refreshing = snapshot
                .servers
                .iter()
                .any(|s| s.in_flight || s.cpu_in_flight);
            (jobs, cpu_jobs)
        };
        if !jobs.is_empty() || !cpu_jobs.is_empty() {
            publish(&app);
        }
        for (config, selection) in jobs {
            tauri::async_runtime::spawn(sample_server(app.clone(), config, selection));
        }
        for config in cpu_jobs {
            tauri::async_runtime::spawn(sample_cpu(app.clone(), config));
        }
        #[cfg(target_os = "macos")]
        tray_status::update(&app);
        let _ = tokio::time::timeout(Duration::from_secs(1), state.refresh.notified()).await;
    }
}

/// One bounded request per server at a time; slow servers never block other cards.
async fn sample_server(
    app: tauri::AppHandle,
    config: ServerConfig,
    selection: Option<Vec<String>>,
) {
    let state = app.state::<AppState>();
    let result = config.query_gpus(&state.home, selection.as_deref()).await;
    {
        let mut snapshot = state.snapshot.lock().unwrap();
        let policies = snapshot.policies.clone();
        let default_policy = snapshot.global.gpu.clone();
        if let Some(server) = snapshot
            .servers
            .iter_mut()
            .find(|s| s.config.id == config.id)
        {
            server.in_flight = false;
            match result {
                Ok(gpus) => {
                    if selection.is_none() {
                        server.gpus = gpus.clone();
                        server
                            .next_poll
                            .retain(|id, _| gpus.iter().any(|g| &g.uuid == id));
                        server
                            .gpu_updated_at
                            .retain(|id, _| gpus.iter().any(|g| &g.uuid == id));
                    }
                    for gpu in gpus {
                        let policy = schedule::resolve_policy(
                            &default_policy,
                            &policies,
                            &config.id,
                            &gpu.uuid,
                        );
                        let interval = policy.seconds(gpu.utilization);
                        server.next_poll.insert(gpu.uuid.clone(), now() + interval);
                        server.gpu_updated_at.insert(gpu.uuid.clone(), now());
                        history::record(
                            &mut server.history,
                            std::slice::from_ref(&gpu),
                            now(),
                            true,
                            interval,
                            policy.history_minutes * 60,
                        );
                        if let Some(existing) = server.gpus.iter_mut().find(|g| g.uuid == gpu.uuid)
                        {
                            *existing = gpu;
                        }
                    }
                    server.status = "online".into();
                    server.error = None;
                    server.updated_at = Some(now());
                    server.retry_at = 0;
                }
                Err(error) => {
                    server.status = "error".into();
                    server.error = Some(error);
                    server.retry_at = now() + 30;
                    server.full_refresh = true;
                    for gpu in &server.gpus {
                        let policy = schedule::resolve_policy(
                            &default_policy,
                            &policies,
                            &config.id,
                            &gpu.uuid,
                        );
                        history::record(
                            &mut server.history,
                            std::slice::from_ref(gpu),
                            now(),
                            false,
                            30,
                            policy.history_minutes * 60,
                        );
                    }
                }
            }
        }
        snapshot.refreshing = snapshot
            .servers
            .iter()
            .any(|s| s.in_flight || s.cpu_in_flight);
    }
    save_history(&state);
    publish(&app);
    state.refresh.notify_one();
}

async fn sample_cpu(app: tauri::AppHandle, config: ServerConfig) {
    let state = app.state::<AppState>();
    let result = config.query_cpu(&state.home).await;
    {
        let mut snapshot = state.snapshot.lock().unwrap();
        let monitor = snapshot.monitor(&config.id);
        if let Some(server) = snapshot
            .servers
            .iter_mut()
            .find(|s| s.config.id == config.id)
        {
            server.cpu_in_flight = false;
            match result {
                Ok(cpu) => {
                    let gpu = ssh::Gpu {
                        index: 0,
                        uuid: "CPU".into(),
                        name: "CPU".into(),
                        utilization: cpu.utilization,
                        memory_used: Some(cpu.memory_used),
                        memory_total: Some(cpu.memory_total),
                        unified_memory: false,
                        temperature: None,
                    };
                    history::record(
                        &mut server.history,
                        &[gpu],
                        now(),
                        true,
                        monitor.cpu_seconds,
                        monitor.cpu_history_minutes * 60,
                    );
                    server.cpu = Some(cpu);
                    server.cpu_error = None;
                    server.cpu_next_poll = now() + monitor.cpu_seconds;
                }
                Err(error) => {
                    server.cpu_error = Some(error);
                    server.cpu_next_poll = now() + 30;
                    let gpu = ssh::Gpu {
                        index: 0,
                        uuid: "CPU".into(),
                        name: "CPU".into(),
                        utilization: None,
                        memory_used: None,
                        memory_total: server.cpu.as_ref().map(|c| c.memory_total),
                        unified_memory: false,
                        temperature: None,
                    };
                    history::record(
                        &mut server.history,
                        &[gpu],
                        now(),
                        false,
                        30,
                        monitor.cpu_history_minutes * 60,
                    );
                }
            }
        }
        snapshot.refreshing = snapshot
            .servers
            .iter()
            .any(|s| s.in_flight || s.cpu_in_flight);
    }
    save_history(&state);
    publish(&app);
    state.refresh.notify_one();
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        .setup(|app| {
            let directory = app.path().app_config_dir()?;
            fs::create_dir_all(&directory)?;
            let config_path = directory.join("servers.json");
            let config: Config = match fs::read(&config_path) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
                Err(e) => return Err(e.into()),
            };
            if config.servers.len() > 32 {
                return Err("Invalid saved settings".into());
            }
            config.global.validate()?;
            config.overlay.validate()?;
            for policy in config.policies.values() {
                policy.validate()?;
            }
            for monitor in config.monitors.values() {
                monitor.validate()?;
            }
            let history_path = directory.join("history.json");
            let (mut archive, history_warning) = match history::load(&history_path, now()) {
                Ok(archive) => (archive, None),
                Err(error) => (history::Archive::new(), Some(error)),
            };
            let mut servers = config
                .servers
                .into_iter()
                .map(ServerView::new)
                .collect::<Result<Vec<_>, _>>()?;
            for server in &mut servers {
                server.history = archive.remove(&server.config.id).unwrap_or_default();
            }
            app.manage(AppState {
                config_path,
                history_path,
                history_io: Mutex::new(()),
                home: app.path().home_dir()?,
                snapshot: Mutex::new(Snapshot {
                    servers,
                    interval: config.interval,
                    refreshing: false,
                    history_warning,
                    policies: config.policies,
                    monitors: config.monitors,
                    global: config.global,
                    overlay: config.overlay,
                    overlay_editing: false,
                    overlay_layout: config.overlay_layout,
                }),
                refresh: Notify::new(),
                setup: tokio::sync::Mutex::new(()),
                pending: Mutex::new(None),
            });
            let show = MenuItem::with_id(app, "show", "打开 GPU Monitor", true, None::<&str>)?;
            let refresh = MenuItem::with_id(app, "refresh", "立即刷新", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let overlay_toggle = MenuItem::with_id(
                app,
                "overlay-toggle",
                "显示 / 隐藏透明浮窗",
                true,
                None::<&str>,
            )?;
            let overlay_edit =
                MenuItem::with_id(app, "overlay-edit", "调整浮窗宽度", true, None::<&str>)?;
            let overlay_lock = MenuItem::with_id(
                app,
                "overlay-lock",
                "完成调整，恢复鼠标穿透",
                true,
                None::<&str>,
            )?;
            let menu = Menu::with_items(
                app,
                &[
                    &show,
                    &overlay_toggle,
                    &overlay_edit,
                    &overlay_lock,
                    &refresh,
                    &quit,
                ],
            )?;
            // Windows keeps the chip glyph; macOS replaces it with live status dots below.
            let mut pixels = vec![0u8; 32 * 32 * 4];
            for y in 0..32 {
                for x in 0..32 {
                    let body = (7..25).contains(&x)
                        && (7..25).contains(&y)
                        && (!(10..22).contains(&x) || !(10..22).contains(&y));
                    let pins = ((3..29).contains(&x) && [9, 10, 15, 16, 21, 22].contains(&y))
                        || ((3..29).contains(&y) && [9, 10, 15, 16, 21, 22].contains(&x));
                    if body || pins {
                        let i = (y * 32 + x) * 4;
                        pixels[i..i + 4].copy_from_slice(&[180, 225, 205, 255]);
                    }
                }
            }
            TrayIconBuilder::with_id("gpu")
                .icon(tauri::image::Image::new_owned(pixels, 32, 32))
                .icon_as_template(!cfg!(target_os = "macos"))
                .tooltip("GPU Monitor")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => show_window(app),
                    "refresh" => request_refresh(&app.state::<AppState>()),
                    "overlay-toggle" => {
                        let mut settings = app
                            .state::<AppState>()
                            .snapshot
                            .lock()
                            .unwrap()
                            .overlay
                            .clone();
                        settings.enabled = !settings.enabled;
                        let _ = set_overlay_settings(app.clone(), settings);
                    }
                    "overlay-edit" => {
                        let _ = begin_overlay_edit(app.clone());
                    }
                    "overlay-lock" => {
                        let _ = end_overlay_edit(app.clone());
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                if let Ok(scale) = window.scale_factor() {
                                    let p = rect.position.to_physical::<f64>(scale);
                                    let size = rect.size.to_physical::<f64>(scale);
                                    if let Ok(ws) = window.outer_size() {
                                        #[cfg(target_os = "macos")]
                                        let y = p.y + size.height + 6.0;
                                        #[cfg(not(target_os = "macos"))]
                                        let y = (p.y - ws.height as f64 - 8.0).max(0.0);
                                        let _ = size;
                                        let _ = window.set_position(tauri::PhysicalPosition::new(
                                            (p.x - ws.width as f64 + 32.0).max(0.0) as i32,
                                            y as i32,
                                        ));
                                    }
                                }
                                show_window(app);
                            }
                        }
                    }
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Regular);
                if let Some(window) = app.get_webview_window("main") {
                    window.set_visible_on_all_workspaces(true)?;
                }
                app.manage(tray_status::State::default());
                tray_status::update(app.handle());
            }
            let (overlay_settings, overlay_layout) = {
                let state = app.state::<AppState>();
                let snapshot = state.snapshot.lock().unwrap();
                (snapshot.overlay.clone(), snapshot.overlay_layout.clone())
            };
            overlay::create(app.handle(), &overlay_settings, overlay_layout.as_ref())?;
            tauri::async_runtime::spawn(poll(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            overlay::on_window_event(window, event);
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "overlay" || window.label() == "overlay-handle" {
                    let app = window.app_handle();
                    let mut settings = app
                        .state::<AppState>()
                        .snapshot
                        .lock()
                        .unwrap()
                        .overlay
                        .clone();
                    settings.enabled = false;
                    let _ = set_overlay_settings(app.clone(), settings);
                }
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            add_connection,
            add_server,
            add_local,
            rename_server,
            remove_server,
            set_refresh_policy,
            set_monitor,
            set_monitor_switches,
            set_cpu_settings,
            reset_monitor_section,
            set_global_settings,
            reset_refresh_policy,
            reset_monitor,
            refresh_now,
            inspect_key,
            get_auth_prompt,
            answer_prompt,
            setup_server,
            set_overlay_settings,
            begin_overlay_edit,
            end_overlay_edit,
            open_main_window,
            overlay::resize_overlay_content
        ])
        .build(tauri::generate_context!())
        .expect("GPU Monitor failed to start")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_window(_app);
            }
        });
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn local_sources_need_no_ssh_and_legacy_connections_still_load() {
        let remote: ServerConfig = serde_json::from_str(
            r#"{"id":"remote","name":"Example","sshCommand":"ssh alice@example.com"}"#,
        )
        .unwrap();
        assert_eq!(remote.source, Source::Ssh);
        assert_eq!(ServerView::new(remote).unwrap().target, "alice@example.com");

        let local: ServerConfig =
            serde_json::from_str(r#"{"id":"local","name":"本机","source":"local"}"#).unwrap();
        let restored: ServerConfig =
            serde_json::from_slice(&serde_json::to_vec(&local).unwrap()).unwrap();
        assert_eq!(restored.source, Source::Local);
        assert!(restored.ssh_command.is_empty());
        assert_eq!(ServerView::new(restored).unwrap().target, "本机");

        let mut snapshot = snapshot();
        snapshot.servers.push(ServerView::new(local).unwrap());
        let mut global = snapshot.global.clone();
        global.monitor.cpu_enabled = true;
        global.gpu.fast_seconds = 3;
        snapshot.apply_global(global, 100);
        assert!(!snapshot.monitor("local").cpu_enabled);
        assert!(snapshot.monitor("inherited").cpu_enabled);
        snapshot
            .set_monitor_switches("local", false, true, 200)
            .unwrap();
        assert!(!snapshot.monitor("local").cpu_enabled);
        assert!(!snapshot.monitor("local").gpu_enabled);
        snapshot.reset_server_monitor("local", 300).unwrap();
        assert!(!snapshot.monitor("local").cpu_enabled);
        assert!(snapshot.monitor("local").gpu_enabled);
    }

    pub(super) fn snapshot() -> Snapshot {
        let mut servers = vec![];
        for id in ["inherited", "custom"] {
            let mut server = ServerView::new(ServerConfig {
                id: id.into(),
                name: id.into(),
                source: Source::Ssh,
                ssh_command: "ssh test-host".into(),
            })
            .unwrap();
            server.gpus = ["GPU-default", "GPU-custom"]
                .iter()
                .enumerate()
                .map(|(index, uuid)| ssh::Gpu {
                    index: index as u32,
                    uuid: (*uuid).into(),
                    name: "test".into(),
                    utilization: Some(0.0),
                    memory_used: Some(0.0),
                    memory_total: Some(100.0),
                    unified_memory: false,
                    temperature: None,
                })
                .collect();
            server.next_poll = server
                .gpus
                .iter()
                .map(|gpu| (gpu.uuid.clone(), 999))
                .collect();
            server.cpu_next_poll = 888;
            server.full_refresh = false;
            servers.push(server);
        }
        Snapshot {
            servers,
            interval: 15,
            refreshing: false,
            history_warning: None,
            policies: BTreeMap::new(),
            monitors: BTreeMap::new(),
            global: schedule::Global::default(),
            overlay: overlay::Settings::default(),
            overlay_editing: false,
            overlay_layout: None,
        }
    }

    #[test]
    fn global_changes_reschedule_inherited_resources_and_preserve_overrides() {
        let mut snapshot = snapshot();
        let custom_gpu = schedule::Policy {
            fixed_seconds: 9,
            mode: "fixed".into(),
            history_minutes: 180,
            ..Default::default()
        };
        snapshot
            .policies
            .insert(schedule::key("inherited", "GPU-custom"), custom_gpu.clone());
        let custom_monitor = schedule::Monitor {
            cpu_enabled: false,
            cpu_seconds: 50,
            ..Default::default()
        };
        snapshot
            .monitors
            .insert("custom".into(), custom_monitor.clone().into());
        let mut global = schedule::Global::default();
        global.gpu.fast_seconds = 3;
        global.gpu.history_minutes = 360;
        global.monitor.cpu_enabled = true;
        global.monitor.cpu_seconds = 7;
        snapshot.apply_global(global.clone(), 100);
        assert_eq!(snapshot.servers[0].next_poll["GPU-default"], 100);
        assert_eq!(snapshot.servers[0].next_poll["GPU-custom"], 999);
        assert_eq!(snapshot.servers[0].cpu_next_poll, 100);
        assert_eq!(snapshot.servers[1].cpu_next_poll, 888);
        assert_eq!(snapshot.monitor("inherited"), global.monitor);
        assert_eq!(snapshot.monitor("custom"), custom_monitor);
        assert_eq!(
            schedule::resolve_policy(
                &snapshot.global.gpu,
                &snapshot.policies,
                "inherited",
                "GPU-custom"
            ),
            &custom_gpu
        );
        assert_eq!(
            schedule::resolve_policy(
                &snapshot.global.gpu,
                &snapshot.policies,
                "inherited",
                "GPU-default"
            ),
            &global.gpu
        );

        snapshot.reset_gpu("inherited", "GPU-custom", 200).unwrap();
        snapshot.reset_server_monitor("custom", 200).unwrap();
        assert_eq!(
            schedule::resolve_policy(
                &snapshot.global.gpu,
                &snapshot.policies,
                "inherited",
                "GPU-custom"
            ),
            &global.gpu
        );
        assert_eq!(snapshot.monitor("custom"), global.monitor);
        assert_eq!(snapshot.servers[0].next_poll["GPU-custom"], 200);
        assert_eq!(snapshot.servers[1].cpu_next_poll, 200);
    }

    #[test]
    fn legacy_config_migrates_and_explicit_settings_survive_serialization() {
        let mut config: Config = serde_json::from_str(r#"{"servers":[],"interval":10}"#).unwrap();
        assert_eq!(config.global, schedule::Global::default());
        config.global.gpu.fast_seconds = 2;
        config.policies.insert(
            "server/GPU".into(),
            schedule::Policy {
                history_minutes: 1440,
                ..Default::default()
            },
        );
        let restored: Config =
            serde_json::from_slice(&serde_json::to_vec(&config).unwrap()).unwrap();
        assert_eq!(restored.global, config.global);
        assert_eq!(restored.policies, config.policies);

        let old: Config = serde_json::from_str(r#"{"servers":[],"interval":15,"monitors":{"server":{"cpuEnabled":false,"gpuEnabled":true,"cpuSeconds":5,"cpuHistoryMinutes":15}}}"#).unwrap();
        assert_eq!(old.monitors["server"].cpu_enabled, Some(false));
        assert_eq!(old.monitors["server"].cpu_seconds, Some(5));
        let restored: Config = serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(restored.monitors, old.monitors);
    }

    #[test]
    fn separate_monitor_panels_save_and_reset_without_changing_each_other() {
        let mut snapshot = snapshot();
        snapshot.set_cpu_settings("custom", 9, 180, 100).unwrap();
        assert_eq!(snapshot.monitors["custom"].gpu_enabled, None);
        assert_eq!(snapshot.monitors["custom"].cpu_enabled, None);
        assert!(!snapshot.servers[1].full_refresh);
        assert_eq!(snapshot.servers[1].next_poll["GPU-default"], 999);
        snapshot
            .set_monitor_switches("custom", false, true, 110)
            .unwrap();
        assert_eq!(snapshot.monitor("custom").cpu_seconds, 9);
        assert_eq!(snapshot.monitor("custom").cpu_history_minutes, 180);
        snapshot
            .reset_monitor_section("custom", "cpu", 120)
            .unwrap();
        let current = snapshot.monitor("custom");
        assert!(!current.gpu_enabled);
        assert!(current.cpu_enabled);
        assert_eq!(current.cpu_seconds, 15);
        assert_eq!(current.cpu_history_minutes, 60);
        let saved = serde_json::to_value(&snapshot.monitors["custom"]).unwrap();
        assert!(saved.get("cpuSeconds").is_none());
        assert_eq!(saved["gpuEnabled"], false);
        snapshot
            .reset_monitor_section("custom", "switches", 130)
            .unwrap();
        assert!(!snapshot.monitors.contains_key("custom"));
        assert_eq!(snapshot.monitor("custom"), snapshot.global.monitor);
        assert!(snapshot.servers[1].full_refresh);
        assert!(snapshot.set_cpu_settings("custom", 0, 60, 140).is_err());
        assert!(!snapshot.monitors.contains_key("custom"));
    }

    #[test]
    fn partial_overrides_follow_unrelated_global_changes() {
        let mut snapshot = snapshot();
        snapshot.set_cpu_settings("custom", 9, 180, 100).unwrap();
        snapshot
            .set_monitor_switches("inherited", true, false, 100)
            .unwrap();
        let mut global = snapshot.global.clone();
        global.monitor.gpu_enabled = false;
        global.monitor.cpu_enabled = true;
        global.monitor.cpu_seconds = 45;
        global.monitor.cpu_history_minutes = 360;
        snapshot.apply_global(global, 200);
        assert!(!snapshot.monitor("custom").gpu_enabled);
        assert!(snapshot.monitor("custom").cpu_enabled);
        assert_eq!(snapshot.monitor("custom").cpu_seconds, 9);
        assert_eq!(snapshot.monitor("custom").cpu_history_minutes, 180);
        assert!(snapshot.monitor("inherited").gpu_enabled);
        assert!(!snapshot.monitor("inherited").cpu_enabled);
        assert_eq!(snapshot.monitor("inherited").cpu_seconds, 45);
        assert_eq!(snapshot.servers[0].next_poll["GPU-default"], 999);
        assert_eq!(snapshot.servers[1].next_poll["GPU-default"], 999);
        assert!(!snapshot.servers[0].full_refresh);
        assert!(!snapshot.servers[1].full_refresh);
    }
}
