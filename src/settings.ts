import { effectiveMonitor, effectiveOverlay, effectivePolicy, globalSettings, hasMonitorOverride, escapeHtml as esc, type MonitorPolicy, type RefreshPolicy, type Snapshot } from "./model.ts";

export function historyLabel(minutes: number): string {
  return minutes >= 60 ? `${minutes / 60} 小时` : `${minutes} 分钟`;
}

function historyOptions(selected: number): string {
  return [15, 30, 60, 180, 360, 1440].map(minutes => `<option value="${minutes}" ${minutes === selected ? "selected" : ""}>${historyLabel(minutes)}</option>`).join("");
}

function monitorFields(monitor: MonitorPolicy, includeCpu = true): string {
  return `<div class="monitor-toggles"><label><input name="gpuEnabled" type="checkbox" ${monitor.gpuEnabled ? "checked" : ""}/> GPU</label>${includeCpu ? `<label><input name="cpuEnabled" type="checkbox" ${monitor.cpuEnabled ? "checked" : ""}/> CPU</label>` : ""}</div>`;
}

function cpuFields(monitor: MonitorPolicy): string {
  return `<p class="panel-description">CPU 利用率与系统内存</p><div class="settings-two"><label class="field">刷新间隔 <span>秒</span><input name="cpuSeconds" aria-label="CPU 刷新间隔（秒）" type="number" min="2" max="3600" required value="${monitor.cpuSeconds}"/></label><label class="field">历史长度<select name="cpuHistoryMinutes" aria-label="CPU 历史长度">${historyOptions(monitor.cpuHistoryMinutes)}</select></label></div>`;
}

function resourcePanels(scope: string, gpu: string, cpu: string): string {
  const id = esc(scope);
  return `<div class="resource-settings" data-resource-scope="${id}"><div class="resource-tabs" role="tablist" aria-label="监控内容设置">
    <button type="button" role="tab" id="${id}-gpu-tab" aria-controls="${id}-gpu-panel" aria-selected="true" data-resource-tab="gpu">GPU 设置</button>
    <button type="button" role="tab" id="${id}-cpu-tab" aria-controls="${id}-cpu-panel" aria-selected="false" tabindex="-1" data-resource-tab="cpu">CPU 设置</button>
    </div><div role="tabpanel" id="${id}-gpu-panel" aria-labelledby="${id}-gpu-tab" data-resource-panel="gpu">${gpu}</div><div role="tabpanel" id="${id}-cpu-panel" aria-labelledby="${id}-cpu-tab" data-resource-panel="cpu" hidden>${cpu}</div></div>`;
}

function policyFields(policy: RefreshPolicy): string {
  return `<div class="settings-two"><label class="field">刷新方式<select name="mode"><option value="adaptive" ${policy.mode === "adaptive" ? "selected" : ""}>自动调节</option><option value="fixed" ${policy.mode === "fixed" ? "selected" : ""}>固定间隔</option></select></label><label class="field">历史长度<select name="historyMinutes">${historyOptions(policy.historyMinutes)}</select></label></div>
    <div class="settings-three" data-adaptive ${policy.mode === "fixed" ? "hidden" : ""}><label class="field">利用率 &lt;10%<input aria-label="低利用率间隔（秒）" name="fastSeconds" type="number" required min="2" max="3600" value="${policy.fastSeconds}" ${policy.mode === "fixed" ? "disabled" : ""}/></label><label class="field">10%–&lt;60%<input aria-label="中利用率间隔（秒）" name="normalSeconds" type="number" required min="2" max="3600" value="${policy.normalSeconds}" ${policy.mode === "fixed" ? "disabled" : ""}/></label><label class="field">利用率 ≥60%<input aria-label="高利用率间隔（秒）" name="slowSeconds" type="number" required min="2" max="3600" value="${policy.slowSeconds}" ${policy.mode === "fixed" ? "disabled" : ""}/></label></div>
    <label class="field" data-fixed ${policy.mode === "adaptive" ? "hidden" : ""}>固定间隔 <span>秒</span><input name="fixedSeconds" type="number" required min="2" max="3600" value="${policy.fixedSeconds}" ${policy.mode === "adaptive" ? "disabled" : ""}/></label>`;
}

function sourceLabel(overridden: boolean): string {
  return `<span class="settings-source ${overridden ? "overridden" : ""}">${overridden ? "单独设置" : "跟随全局"}</span>`;
}

function saveControls(overridden: boolean): string {
  return `<div class="settings-save"><span>${overridden ? '<button class="text-button" type="button" data-reset>恢复跟随全局</button>' : '<small>保存后仅此项单独生效</small>'}</span><button class="secondary" type="submit">保存单独设置</button></div>`;
}

function overlayContent(snapshot: Snapshot): string {
  const overlay = effectiveOverlay(snapshot);
  const missingServer = overlay.serverId && !snapshot.servers.some(server => server.id === overlay.serverId);
  return `<section class="overlay-settings"><h2>透明浮窗</h2><form class="settings-card" data-kind="overlay" data-settings-key="overlay">
    <div class="monitor-toggles"><label><input name="enabled" type="checkbox" ${overlay.enabled ? "checked" : ""}/> 显示置顶浮窗</label></div>
    <label class="field range-field">文字不透明度 <output data-range-output="opacity">${overlay.opacity}%</output><input name="opacity" type="range" min="30" max="100" step="1" value="${overlay.opacity}"/></label>
    <label class="field range-field">背景不透明度 <output data-range-output="backgroundOpacity">${overlay.backgroundOpacity}%</output><input name="backgroundOpacity" type="range" min="0" max="80" step="1" value="${overlay.backgroundOpacity}"/></label>
    <div class="settings-two"><label class="field">字号 <span>px</span><input name="fontSize" type="number" min="12" max="28" step="1" required value="${overlay.fontSize}"/></label><label class="field">显示设备<select name="serverId"><option value="" ${overlay.serverId === null ? "selected" : ""}>全部设备</option>${snapshot.servers.map(server => `<option value="${esc(server.id)}" ${server.id === overlay.serverId ? "selected" : ""}>${esc(server.name)}</option>`).join("")}${missingServer ? `<option value="${esc(overlay.serverId!)}" selected disabled>原设备已移除，请重选</option>` : ""}</select></label></div>
    <p class="form-note overlay-note">背景设为 0% 即完全透明。平时只显示内容；鼠标移入面板时，内容隐藏并出现可拖动的小柄。移出面板范围后，内容恢复、小柄隐藏。除小柄外，点击与滚轮直接作用于下方页面。</p>
    <div class="settings-save"><small data-overlay-mode>${snapshot.overlayEditing ? "调整中 · 暂停自动隐藏" : overlay.enabled ? "鼠标移入时隐藏" : "浮窗已关闭"}</small><button class="secondary" type="submit">保存浮窗设置</button></div>
  </form><div class="overlay-adjust"><button class="secondary" type="button" data-overlay-edit ${!overlay.enabled && !snapshot.overlayEditing ? "disabled" : ""}>${snapshot.overlayEditing ? "完成调整，恢复穿透" : "调整宽度"}</button><p class="form-note">边缘调整宽度，高度随内容自动变化。完成后恢复自动隐藏与穿透。</p></div></section>`;
}

export function settingsContent(snapshot: Snapshot): string {
  const global = globalSettings(snapshot);
  const globalGpu = `<form class="settings-card resource-card" data-kind="global-gpu" data-settings-key="global-gpu"><h3>GPU 刷新与历史</h3>${policyFields(global.gpu)}<div class="settings-save"><small>间隔单位：秒 · 范围 2–3600</small><button class="secondary" type="submit">保存 GPU 设置</button></div></form>`;
  const globalCpu = `<form class="settings-card resource-card" data-kind="global-cpu" data-settings-key="global-cpu"><h3>CPU 刷新与历史</h3>${cpuFields(global.monitor)}<div class="settings-save"><small>CPU 关闭时仍保留此设置</small><button class="secondary" type="submit">保存 CPU 设置</button></div></form>`;
  const globalContent = `<section class="settings-global"><h2>全局默认</h2><p class="form-note">未单独设置的设备与 GPU 自动跟随。单独设置的项保持自己的配置。</p>
    <form class="settings-card monitor-choice" data-kind="global-monitor" data-settings-key="global-monitor"><h3>监控内容</h3>${monitorFields(global.monitor)}<div class="settings-save"><small>关闭对应项会停止后续采集</small><button class="secondary" type="submit">保存监控内容</button></div></form>
    ${resourcePanels("global", globalGpu, globalCpu)}
  </section>`;
  if (!snapshot.servers.length) return overlayContent(snapshot) + globalContent + '<p class="muted">添加设备后，可在这里为单台设备或单张 GPU 设置例外。</p>';
  return overlayContent(snapshot) + globalContent + '<h2 class="overrides-heading">单独设置</h2>' + snapshot.servers.map(server => {
    const local = server.source === "local";
    const monitor = effectiveMonitor(snapshot, server.id);
    const monitorOverride = hasMonitorOverride(snapshot, server.id, "switches");
    const cpuOverride = hasMonitorOverride(snapshot, server.id, "cpu");
    const gpuPanel = server.gpus.map(gpu => {
        const policy = effectivePolicy(snapshot, server.id, gpu.uuid);
        const overridden = Boolean(snapshot.policies?.[`${server.id}/${gpu.uuid}`]);
        return `<form class="settings-card" data-kind="gpu" data-server="${esc(server.id)}" data-gpu="${esc(gpu.uuid)}" data-settings-key="gpu/${esc(server.id)}/${esc(gpu.uuid)}">
          <div class="settings-card-heading"><h3>GPU ${gpu.index} · ${esc(gpu.name.replace(/^NVIDIA /, ""))}</h3>${sourceLabel(overridden)}</div>
          ${policyFields(policy)}${saveControls(overridden)}
        </form>`;
      }).join("") || `<p class="form-note">${local ? "检测到本机 GPU 后，可逐卡设置。" : "开启 GPU 监控并连接成功后，可逐卡设置。"}</p>`;
    const cpuPanel = `<form class="settings-card resource-card" data-kind="cpu" data-server="${esc(server.id)}" data-settings-key="cpu/${esc(server.id)}"><div class="settings-card-heading"><h3>CPU 刷新与历史</h3>${sourceLabel(cpuOverride)}</div>${cpuFields(monitor)}${saveControls(cpuOverride)}</form>`;
    return `<details class="settings-server" data-settings-server="${esc(server.id)}"><summary>${esc(server.name)}<small>${server.gpus.length} 张 GPU</small></summary>
      <form class="settings-card monitor-choice" data-kind="monitor" data-server="${esc(server.id)}" data-settings-key="monitor/${esc(server.id)}">
        <div class="settings-card-heading"><h3>监控内容</h3>${sourceLabel(monitorOverride)}</div>${monitorFields(monitor, !local)}${saveControls(monitorOverride)}
      </form>
      ${local ? gpuPanel : resourcePanels(`server-${server.id}`, gpuPanel, cpuPanel)}
    </details>`;
  }).join("");
}
