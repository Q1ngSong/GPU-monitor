use serde::{Deserialize, Serialize};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const MIN_CONTENT_HEIGHT: f64 = 44.0;
const MAX_CONTENT_HEIGHT: f64 = 16384.0;

#[derive(Default)]
struct Interaction {
    layout_changed: Mutex<Option<Instant>>,
    hover: Mutex<HoverState>,
}

#[derive(Default)]
struct HoverState {
    hidden: bool,
    away_since: Option<Instant>,
}

impl HoverState {
    fn update(&mut self, inside: bool, editing: bool, now: Instant) -> bool {
        if editing {
            self.hidden = false;
            self.away_since = None;
        } else if inside {
            self.hidden = true;
            self.away_since = None;
        } else if self.hidden {
            // Keep the handle stable across brief border crossings during a native drag.
            let away = self.away_since.get_or_insert(now);
            if now.duration_since(*away) >= Duration::from_millis(120) {
                self.hidden = false;
                self.away_since = None;
            }
        }
        self.hidden
    }
}

fn align_with(source: &WebviewWindow, target: &WebviewWindow) -> tauri::Result<()> {
    let position = source.outer_position()?;
    #[cfg(not(target_os = "macos"))]
    if target.outer_position()? != position {
        target.set_position(position)?;
    }
    #[cfg(target_os = "macos")]
    {
        // macOS reports each window in its own screen's pixel scale.
        let position = position.to_logical::<f64>(source.scale_factor()?);
        let current = target
            .outer_position()?
            .to_logical::<f64>(target.scale_factor()?);
        if (current.x - position.x).abs() > 0.5 || (current.y - position.y).abs() > 0.5 {
            target.set_position(position)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub opacity: u8,
    pub background_opacity: u8,
    pub font_size: u8,
    pub server_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            opacity: 100,
            background_opacity: 0,
            font_size: 16,
            server_id: None,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(30..=100).contains(&self.opacity)
            || self.background_opacity > 80
            || !(12..=28).contains(&self.font_size)
        {
            return Err(
                "文字不透明度应为 30–100%，背景不透明度应为 0–80%，字号应为 12–28。".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layout {
    x: i32,
    y: i32,
    width: f64,
    height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position_scale: Option<f64>,
}

#[cfg(any(not(target_os = "macos"), test))]
fn fit(position: i32, origin: i32, area: u32, extent: u32) -> i32 {
    let end = i64::from(origin) + i64::from(area.saturating_sub(extent));
    i64::from(position).clamp(i64::from(origin), end) as i32
}

#[cfg(not(target_os = "macos"))]
fn placement(
    window: &WebviewWindow,
    saved: Option<&Layout>,
    requested_size: Option<tauri::LogicalSize<f64>>,
) -> Result<tauri::Position, String> {
    let monitor = saved
        .and_then(|layout| {
            window
                .monitor_from_point(layout.x as f64, layout.y as f64)
                .ok()
                .flatten()
        })
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("找不到浮窗所在屏幕。")?;
    let area = monitor.work_area();
    let size = match requested_size {
        Some(size) => size.to_physical::<u32>(window.scale_factor().map_err(|e| e.to_string())?),
        None => window.outer_size().map_err(|e| e.to_string())?,
    };
    let margin = (24.0 * monitor.scale_factor()) as i32;
    let desired = saved.map(|layout| (layout.x, layout.y)).unwrap_or((
        area.position.x + area.size.width as i32 - size.width as i32 - margin,
        area.position.y + margin,
    ));
    Ok(PhysicalPosition::new(
        fit(desired.0, area.position.x, area.size.width, size.width),
        fit(desired.1, area.position.y, area.size.height, size.height),
    )
    .into())
}

#[cfg(any(target_os = "macos", test))]
fn logical_position(layout: &Layout, legacy_scale: f64) -> tauri::LogicalPosition<f64> {
    let scale = layout
        .position_scale
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(legacy_scale);
    PhysicalPosition::new(layout.x, layout.y).to_logical(scale)
}

#[cfg(any(target_os = "macos", test))]
fn fit_logical(position: f64, origin: f64, area: f64, extent: f64) -> f64 {
    position.clamp(origin, origin + (area - extent).max(0.0))
}

#[cfg(target_os = "macos")]
fn placement(
    window: &WebviewWindow,
    saved: Option<&Layout>,
    requested_size: Option<tauri::LogicalSize<f64>>,
) -> Result<tauri::Position, String> {
    let primary = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("找不到浮窗所在屏幕。")?;
    let desired = saved.map(|layout| {
        // Older layouts have no scale; infer it from the saved screen's physical work area.
        let legacy_scale = window
            .available_monitors()
            .unwrap_or_default()
            .into_iter()
            .find(|monitor| {
                let area = monitor.work_area();
                contains(
                    PhysicalPosition::new(layout.x as f64, layout.y as f64),
                    area.position,
                    area.size,
                )
            })
            .map_or(primary.scale_factor(), |monitor| monitor.scale_factor());
        logical_position(layout, legacy_scale)
    });
    let monitor = desired
        .and_then(|position| {
            window
                .monitor_from_point(position.x, position.y)
                .ok()
                .flatten()
        })
        .unwrap_or(primary);
    let area = monitor.work_area();
    let origin = area.position.to_logical::<f64>(monitor.scale_factor());
    let area_size = area.size.to_logical::<f64>(monitor.scale_factor());
    let size = match requested_size {
        Some(size) => size,
        None => window
            .outer_size()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(window.scale_factor().map_err(|e| e.to_string())?),
    };
    let desired = desired.unwrap_or(tauri::LogicalPosition::new(
        origin.x + area_size.width - size.width - 24.0,
        origin.y + 24.0,
    ));
    Ok(tauri::LogicalPosition::new(
        fit_logical(desired.x, origin.x, area_size.width, size.width),
        fit_logical(desired.y, origin.y, area_size.height, size.height),
    )
    .into())
}

pub fn place(window: &WebviewWindow, saved: Option<&Layout>) -> Result<(), String> {
    window
        .set_position(placement(window, saved, None)?)
        .map_err(|error| error.to_string())
}

fn content_size(
    current: tauri::PhysicalSize<u32>,
    scale: f64,
    height: f64,
) -> Result<Option<tauri::LogicalSize<f64>>, String> {
    if !height.is_finite()
        || height <= 0.0
        || height > MAX_CONTENT_HEIGHT
        || !scale.is_finite()
        || scale <= 0.0
    {
        return Err("浮窗内容高度无效。".into());
    }
    let width = current.to_logical::<f64>(scale).width;
    let size = tauri::LogicalSize::new(width, height.max(MIN_CONTENT_HEIGHT));
    Ok((size.to_physical::<u32>(scale).height != current.height).then_some(size))
}

#[tauri::command]
pub fn resize_overlay_content(window: WebviewWindow, height: f64) -> Result<(), String> {
    if window.label() != "overlay" {
        return Err("仅监控浮窗可以调整内容高度。".into());
    }
    let app = window.app_handle();
    // The webview may measure content while its companion window is still being created.
    let Some(interaction) = app.try_state::<Interaction>() else {
        return Ok(());
    };
    let Some(handle) = app.get_webview_window("overlay-handle") else {
        return Ok(());
    };
    // Do not move the replacement handle during a drag; the renderer retries after reveal.
    if interaction.hover.lock().unwrap().hidden {
        return Ok(());
    }
    let current = window.inner_size().map_err(|error| error.to_string())?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let Some(size) = content_size(current, scale, height)? else {
        return Ok(());
    };
    let layout = capture(app)?;
    let position = placement(&window, Some(&layout), Some(size))?;
    window.set_size(size).map_err(|error| error.to_string())?;
    window
        .set_position(position)
        .map_err(|error| error.to_string())?;
    // macOS applies geometry asynchronously; give both windows the same computed position.
    handle
        .set_position(position)
        .map_err(|error| error.to_string())
}

pub fn create(
    app: &tauri::AppHandle,
    settings: &Settings,
    layout: Option<&Layout>,
) -> Result<(), String> {
    let (width, height) = layout
        .map(|l| {
            (
                l.width.clamp(280.0, 1200.0),
                l.height.clamp(MIN_CONTENT_HEIGHT, MAX_CONTENT_HEIGHT),
            )
        })
        .unwrap_or((620.0, 160.0));
    let builder = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("GPU Monitor 浮窗")
        .inner_size(width, height)
        .min_inner_size(280.0, MIN_CONTENT_HEIGHT)
        .max_inner_size(1200.0, MAX_CONTENT_HEIGHT)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .focused(false)
        .focusable(false)
        .resizable(false)
        .visible(false);
    #[cfg(target_os = "windows")]
    let builder = builder.no_redirection_bitmap(true);
    let window = builder.build().map_err(|e| e.to_string())?;
    place(&window, layout)?;
    // A separate, always interactive handle keeps dragging reliable while the readout ignores input.
    let handle_builder =
        WebviewWindowBuilder::new(app, "overlay-handle", WebviewUrl::App("handle.html".into()))
            .title("GPU Monitor 拖动柄")
            .inner_size(28.0, 44.0)
            .min_inner_size(28.0, 44.0)
            .max_inner_size(28.0, 44.0)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .transparent(true)
            .background_color(tauri::window::Color(0, 0, 0, 0))
            .focused(false)
            .focusable(false)
            .resizable(false)
            .visible(false);
    #[cfg(target_os = "windows")]
    let handle_builder = handle_builder.no_redirection_bitmap(true);
    let handle = handle_builder.build().map_err(|e| e.to_string())?;
    // Windows can impose its default minimum width during the first creation messages.
    handle
        .set_size(tauri::LogicalSize::new(28.0, 44.0))
        .map_err(|e| e.to_string())?;
    align_with(&window, &handle).map_err(|e| e.to_string())?;
    app.manage(Interaction::default());
    apply(app, settings, false)?;
    tauri::async_runtime::spawn(watch(app.clone()));
    Ok(())
}

/// Mouse passthrough is a native window property, including wheel and scrollbar hit testing.
pub fn apply(app: &tauri::AppHandle, settings: &Settings, editing: bool) -> Result<(), String> {
    let window = app.get_webview_window("overlay").ok_or("浮窗尚未创建。")?;
    let handle = app
        .get_webview_window("overlay-handle")
        .ok_or("拖动柄尚未创建。")?;
    let result = (|| {
        *app.state::<Interaction>().hover.lock().unwrap() = HoverState::default();
        align_with(&window, &handle)?;
        window.set_ignore_cursor_events(!editing)?;
        window.set_focusable(editing)?;
        window.set_resizable(editing)?;
        window.set_always_on_top(true)?;
        if settings.enabled {
            window.show()?;
            // The handle replaces the readout only after the pointer enters its area.
            handle.hide()?;
            if editing {
                window.set_focus()?;
            }
        } else {
            window.hide()?;
            handle.hide()?;
        }
        Ok::<(), tauri::Error>(())
    })();
    // Do not leave a supposedly locked overlay intercepting input if native setup failed.
    if result.is_err() {
        let _ = window.hide();
        let _ = handle.hide();
    }
    result.map_err(|e| e.to_string())
}

/// Keep the two native windows together, including moves made by the adjustment toolbar.
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let label = window.label();
    if label != "overlay" && label != "overlay-handle" {
        return;
    }
    let app = window.app_handle();
    let Some(interaction) = app.try_state::<Interaction>() else {
        return;
    };
    match event {
        tauri::WindowEvent::Moved(position) => {
            let editing = app
                .state::<super::AppState>()
                .snapshot
                .lock()
                .unwrap()
                .overlay_editing;
            let driver = if editing { "overlay" } else { "overlay-handle" };
            if label != driver || window.outer_position().ok() != Some(*position) {
                return;
            }
            let other = if editing { "overlay-handle" } else { "overlay" };
            if let (Some(source), Some(target)) = (
                app.get_webview_window(driver),
                app.get_webview_window(other),
            ) {
                let _ = align_with(&source, &target);
            }
            *interaction.layout_changed.lock().unwrap() = Some(Instant::now());
        }
        tauri::WindowEvent::Resized(_) if label == "overlay" => {
            *interaction.layout_changed.lock().unwrap() = Some(Instant::now());
        }
        _ => {}
    }
}

fn contains(
    cursor: PhysicalPosition<f64>,
    position: PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
) -> bool {
    cursor.x >= f64::from(position.x)
        && cursor.y >= f64::from(position.y)
        && cursor.x < f64::from(position.x) + f64::from(size.width)
        && cursor.y < f64::from(position.y) + f64::from(size.height)
}

fn tick(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<super::AppState>();
    let interaction = app.state::<Interaction>();
    let (enabled, editing) = {
        let snapshot = state.snapshot.lock().unwrap();
        (snapshot.overlay.enabled, snapshot.overlay_editing)
    };
    if enabled {
        let body = app.get_webview_window("overlay").ok_or("浮窗尚未创建。")?;
        let handle = app
            .get_webview_window("overlay-handle")
            .ok_or("拖动柄尚未创建。")?;
        // Global coordinates still work when the readout is hidden or ignores cursor events.
        if let Ok(cursor) = app.cursor_position() {
            #[cfg(target_os = "macos")]
            let cursor = {
                let primary = app
                    .primary_monitor()
                    .map_err(|e| e.to_string())?
                    .ok_or("找不到主屏幕。")?;
                let scale =
                    body.scale_factor().map_err(|e| e.to_string())? / primary.scale_factor();
                PhysicalPosition::new(cursor.x * scale, cursor.y * scale)
            };
            let inside = contains(
                cursor,
                body.outer_position().map_err(|e| e.to_string())?,
                body.outer_size().map_err(|e| e.to_string())?,
            );
            let hide = interaction
                .hover
                .lock()
                .unwrap()
                .update(inside, editing, Instant::now());
            let visible = body.is_visible().map_err(|e| e.to_string())?;
            if hide && visible {
                body.hide().map_err(|e| e.to_string())?;
            }
            if !hide && !visible {
                body.show().map_err(|e| e.to_string())?;
            }
            let handle_visible = handle.is_visible().map_err(|e| e.to_string())?;
            if hide && !handle_visible {
                handle.show().map_err(|e| e.to_string())?;
            }
            if !hide && handle_visible {
                handle.hide().map_err(|e| e.to_string())?;
            }
        }
    }
    let save = {
        let mut changed = interaction.layout_changed.lock().unwrap();
        if changed.is_some_and(|at| at.elapsed() >= Duration::from_millis(700)) {
            *changed = None;
            true
        } else {
            false
        }
    };
    if save {
        let layout = capture(app)?;
        let mut snapshot = state.snapshot.lock().unwrap();
        let mut updated = snapshot.clone();
        updated.overlay_layout = Some(layout);
        super::persist(&state, &updated)?;
        *snapshot = updated;
    }
    Ok(())
}

async fn watch(app: tauri::AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_millis(35)).await;
        let (done, complete) = tokio::sync::oneshot::channel();
        let tick_app = app.clone();
        // At most one tick is queued; native move loops cannot accumulate stale hover updates.
        if app
            .run_on_main_thread(move || {
                if let Err(error) = tick(&tick_app) {
                    eprintln!("Overlay: {error}");
                }
                let _ = done.send(());
            })
            .is_err()
        {
            break;
        }
        if complete.await.is_err() {
            break;
        }
    }
}

pub fn capture(app: &tauri::AppHandle) -> Result<Layout, String> {
    let window = app.get_webview_window("overlay").ok_or("浮窗尚未创建。")?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);
    Ok(Layout {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        position_scale: if cfg!(target_os = "macos") {
            Some(scale)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_defaults_and_opacity_limits() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.background_opacity, 0);
        assert!(Settings {
            opacity: 29,
            ..settings.clone()
        }
        .validate()
        .is_err());
        assert!(Settings {
            background_opacity: 81,
            ..settings.clone()
        }
        .validate()
        .is_err());
        assert!(Settings {
            font_size: 29,
            ..settings
        }
        .validate()
        .is_err());
    }

    #[test]
    fn legacy_auto_scroll_is_ignored_without_losing_overlay_settings() {
        let settings: Settings = serde_json::from_str(
            r#"{"autoScroll":false,"enabled":false,"opacity":75,"fontSize":20}"#,
        )
        .unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.opacity, 75);
        assert_eq!(settings.font_size, 20);
        assert_eq!(settings.background_opacity, 0);
        assert!(settings.validate().is_ok());
        assert!(serde_json::to_value(settings)
            .unwrap()
            .get("autoScroll")
            .is_none());
    }

    #[test]
    fn content_height_preserves_width_and_skips_equal_physical_sizes() {
        let current = tauri::PhysicalSize::new(775, 200);
        assert_eq!(content_size(current, 1.25, 160.1).unwrap(), None);
        let size = content_size(current, 1.25, 1200.0).unwrap().unwrap();
        assert_eq!(size.width, 620.0);
        assert_eq!(size.height, 1200.0);
        assert_eq!(size.to_physical::<u32>(1.25).width, current.width);
        assert_eq!(
            content_size(current, 1.25, 12.0).unwrap().unwrap().height,
            MIN_CONTENT_HEIGHT
        );
        assert_eq!(
            content_size(current, 1.25, MAX_CONTENT_HEIGHT)
                .unwrap()
                .unwrap()
                .height,
            MAX_CONTENT_HEIGHT
        );
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -1.0,
            MAX_CONTENT_HEIGHT + 1.0,
        ] {
            assert!(content_size(current, 1.25, invalid).is_err());
        }
    }

    #[test]
    fn placement_preserves_negative_monitor_coordinates_and_recovers_offscreen() {
        assert_eq!(fit(-800, -1920, 1920, 620), -800);
        assert_eq!(fit(800, -1920, 1920, 620), -620);
        assert_eq!(fit(-3000, -1920, 1920, 620), -1920);
        assert_eq!(fit(100, 0, 300, 620), 0);
    }

    #[test]
    fn mac_layout_restores_logical_coordinates_across_display_scales() {
        let saved: Layout = serde_json::from_str(
            r#"{"x":3400,"y":200,"width":620,"height":160,"position_scale":2.0}"#,
        )
        .unwrap();
        assert_eq!(
            logical_position(&saved, 1.0),
            tauri::LogicalPosition::new(1700.0, 100.0)
        );
        assert_eq!(fit_logical(1700.0, 1512.0, 1920.0, 620.0), 1700.0);
        assert_eq!(fit_logical(-800.0, -1920.0, 1920.0, 620.0), -800.0);
        assert_eq!(fit_logical(1700.0, 0.0, 1512.0, 620.0), 892.0);
        assert_eq!(fit_logical(100.0, 0.0, 300.0, 620.0), 0.0);
    }

    #[test]
    fn legacy_mac_layout_uses_the_inferred_screen_scale() {
        let saved: Layout =
            serde_json::from_str(r#"{"x":3400,"y":200,"width":620,"height":160}"#).unwrap();
        assert_eq!(saved.position_scale, None);
        assert_eq!(
            logical_position(&saved, 2.0),
            tauri::LogicalPosition::new(1700.0, 100.0)
        );
    }

    #[test]
    fn pointer_hit_area_includes_the_replacement_handle() {
        // Physical coordinates also cover negative-origin and high-DPI monitors.
        let pos = PhysicalPosition::new(-1200, 100);
        let body = tauri::PhysicalSize::new(930, 240);
        let inside = |x, y| contains(PhysicalPosition::new(x, y), pos, body);
        assert!(inside(-1180.0, 120.0));
        assert!(inside(-1100.0, 120.0));
        assert!(inside(-1180.0, 200.0));
        assert!(!inside(-1250.0, 120.0));
        assert!(!inside(-270.0, 120.0));
        assert!(!inside(-1100.0, 340.0));
    }

    #[test]
    fn handle_only_replaces_hidden_panel_and_does_not_flicker_during_drag() {
        let mut hover = HoverState::default();
        let now = Instant::now();
        assert!(!hover.update(false, false, now));
        assert!(hover.update(true, false, now));
        assert!(hover.update(false, false, now));
        assert!(hover.update(false, false, now + Duration::from_millis(80)));
        assert!(hover.update(true, false, now + Duration::from_millis(100)));
        assert!(hover.update(false, false, now + Duration::from_millis(200)));
        assert!(!hover.update(false, false, now + Duration::from_millis(330)));
        assert!(hover.update(true, false, now + Duration::from_millis(400)));
        assert!(!hover.update(true, true, now + Duration::from_millis(410)));
    }
}
