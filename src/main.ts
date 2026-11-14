import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyOverlayEdits, applyPolicyEdits, chartSegments, effectiveMonitor, effectiveOverlay, effectivePolicy, globalSettings, updateMonitorOverride, escapeHtml as esc, gib, isAvailable, summary, type Gpu, type Server, type Snapshot, type Sample, type RefreshPolicy, type MonitorPolicy, type MonitorSection, type OverlaySettings } from "./model";
import { historyLabel, settingsContent } from "./settings";
import "./style.css";

const icons = {
  settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3"/><circle cx="15" cy="17" r="3"/>',
  chip: '<rect x="6" y="6" width="12" height="12" rx="3"/><rect x="9" y="9" width="6" height="6" rx="1"/><path d="M9 3v3m6-3v3M9 18v3m6-3v3M3 9h3m-3 6h3m12-6h3m-3 6h3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  refresh: '<path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1"/>',
  close: '<path d="m6 6 12 12M6 18 18 6"/>',
  server: '<rect x="4" y="4" width="16" height="6" rx="2"/><rect x="4" y="14" width="16" height="6" rx="2"/><path d="M7 7h.01M7 17h.01M11 7h6M11 17h6"/>',
  arrow: '<path d="M5 12h14m-6-6 6 6-6 6"/>',
  key: '<circle cx="8" cy="9" r="4"/><path d="m11 12 8 8m-3-3 3-3m-6 0 3-3"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
};
function icon(name: keyof typeof icons, cls = "") { return `<svg class="icon ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name]}</svg>`; }

const desktop = isTauri();
const demo = !desktop && new URLSearchParams(location.search).has("demo");
let snapshot: Snapshot = { servers: [], interval: 15, refreshing: false };
let availableOnly = false;
let addSource: "local" | "ssh" = "local";
let setupId: string | null = null;
let removeId: string | null = null;
let prompt: { id: string; text: string; confirmation: boolean } | null = null;
let toastTimer: ReturnType<typeof setTimeout>;
let settingsSaving = false;
let overlayControlBusy = false;
let lastHistoryWarning: string | null = null;

document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
  <header id="titlebar">
    <div class="brand">${icon("chip")}<span>GPU Monitor</span><span class="brand-dot"></span></div>
    <div class="actions"><button class="icon-button" id="settings" aria-label="设置" aria-pressed="false" aria-controls="settings-page" title="设置">${icon("settings")}</button><button class="icon-button" id="add" aria-label="添加设备" title="添加设备">${icon("plus")}</button><button class="icon-button" id="refresh" aria-label="立即刷新" title="立即刷新">${icon("refresh")}</button><button class="icon-button close-window" id="hide" aria-label="收起到托盘" title="收起到托盘">${icon("close")}</button></div>
  </header>
  <main id="dashboard">
    <section class="overview" aria-label="GPU 概览"><div class="headline"><h1>GPU 状态</h1><span id="live" class="live"><i></i>实时</span></div><div class="stats" id="stats"></div></section>
    <div class="list-toolbar"><div class="filter-controls"><div class="tabs" role="group" aria-label="筛选 GPU"><button id="all-filter" class="active" aria-pressed="true">全部 GPU</button><button id="available-filter" aria-pressed="false">低负载</button></div><button class="help-button" id="load-help" aria-label="低负载是什么意思" title="低负载说明">ⓘ</button></div><span id="server-count"></span></div>
    <div id="servers"></div>
  </main>
  <main id="settings-page" hidden><div class="settings-title"><button id="back-settings" class="icon-button" aria-label="返回监控">${icon("arrow", "back-arrow")}</button><h1>设置</h1></div><p class="muted settings-intro">分别调整浮窗、监控内容和刷新间隔。<br>单独设置优先于全局默认。</p><div id="settings-content"></div><p class="form-note">历史最长保留 24 小时。延长历史窗口后，从已有采样继续积累，不回填过去数据。自动模式按本次利用率决定下次间隔，变化在下次采样时发现。</p></main>
  <div id="toast" role="status" hidden></div>
  <dialog id="load-dialog"><div class="dialog-heading"><h2>什么是低负载？</h2><button class="icon-button" id="close-load" aria-label="关闭说明">${icon("close")}</button></div><p class="muted">一张 GPU 同时满足以下两项时，标记为低负载：</p><ul class="load-explanation"><li>GPU 利用率 <strong>低于 10%</strong></li><li>显存占用 <strong>低于总量的 10%</strong></li></ul><p class="form-note">这可以排除“暂时没在计算，但显存仍被占用”的情况。断线或指标未知的卡不计入低负载。</p><p class="form-note">这是当前采样的参考，不代表资源已预留，也不保证你的任务一定能运行。</p></dialog>
  <dialog id="add-dialog"><form id="add-form">
    <div class="dialog-heading"><h2>添加设备</h2><button type="button" class="icon-button" id="close-add" aria-label="关闭">${icon("close")}</button></div>
    <div class="add-source" role="group" aria-label="监控来源"><button type="button" data-add-source="local" aria-pressed="true">本机</button><button type="button" data-add-source="ssh" aria-pressed="false">SSH 服务器</button></div>
    <p class="muted" id="add-description">查看本机 NVIDIA GPU，无需 SSH 或密码。</p>
    <fieldset id="ssh-fields" disabled hidden><div class="connection-fields"><label class="field">IP / 域名<input id="host" required maxlength="253" placeholder="192.168.1.100" spellcheck="false" autocomplete="off" /></label><label class="field">端口<input id="port" type="number" required min="1" max="65535" value="22" inputmode="numeric" /></label></div><label class="field">用户名<input id="username" required maxlength="64" value="root" placeholder="root" spellcheck="false" autocomplete="off" /></label><label class="field">密码<input id="server-password" type="password" placeholder="已有免密连接时可留空" autocomplete="off" /></label><div id="key-info" class="key-info">${icon("key")}<span>自动检测本机密钥；没有时创建 Ed25519 密钥。</span></div><p class="form-note">密码仅用于首次接入，不会保存。之后使用密钥自动连接。</p></fieldset>
    <p id="form-error" class="error-text" role="alert" hidden></p><button type="submit" class="primary full" id="connect-button">添加本机 ${icon("plus")}</button>
  </form></dialog>
  <dialog id="auth-dialog"><form id="auth-form"><div class="auth-symbol">${icon("key")}</div><h2 id="auth-title">验证 SSH 连接</h2><p id="auth-description" class="muted"></p><pre id="auth-prompt"></pre><label id="password-label" class="field">密码<input id="password" type="password" autocomplete="off" spellcheck="false" /></label><p class="form-note">仅传给本机 SSH 进程，不保存密码。</p><div class="dialog-buttons"><button type="button" class="secondary" id="cancel-auth">取消</button><button type="submit" class="primary" id="submit-auth">继续连接</button></div></form></dialog>
`;

const $ = <T extends HTMLElement = HTMLElement>(selector: string) => document.querySelector<T>(selector)!;

function notify(message: string) {
  clearTimeout(toastTimer);
  $("#toast").textContent = message;
  $("#toast").hidden = false;
  toastTimer = setTimeout(() => { $("#toast").hidden = true; }, 6000);
}

function lineChart(samples: Sample[], metric: "utilization" | "memoryUsed", maximum: number, topLabel: string, minutes: number): string {
  const segments = chartSegments(samples, metric, maximum, Date.now() / 1000, snapshot.interval, minutes * 60);
  const paths = segments.map(points => {
    // Bound SVG path size while retaining each pixel column's low and high points.
    if (points.length > 500) {
      const buckets = new Map<number, Array<[number, number]>>();
      for (const point of points) { const bucket = Math.floor(point[0]); const group = buckets.get(bucket) ?? []; group.push(point); buckets.set(bucket, group); }
      points = [...buckets.values()].flatMap(group => {
        const low = group.reduce((a, b) => a[1] < b[1] ? a : b);
        const high = group.reduce((a, b) => a[1] > b[1] ? a : b);
        return low === high ? [low] : [low, high].sort((a, b) => a[0] - b[0]);
      });
    }
    const line = points.map(([x, y], index) => `${index ? "L" : "M"}${x} ${y}`).join(" ");
    return points.length > 1 ? `<path class="chart-line" d="${line}"/>` : `<circle class="chart-point" cx="${points[0][0]}" cy="${points[0][1]}" r="2"/>`;
  }).join("");
  const title = `最近 ${historyLabel(minutes)}${metric === "utilization" ? "利用率" : "内存占用"}`;
  const short = (m: number) => m >= 60 ? `${m / 60}h` : `${m}m`;
  return `<div class="chart ${metric === "memoryUsed" ? "memory-chart" : "compute-chart"}"><span class="chart-range">${topLabel}</span><svg viewBox="-3 0 166 58" role="img" aria-label="${title}"><title>${title}，断线时曲线留空</title><path class="chart-grid" d="M0 6H160 M0 29H160 M0 52H160"/>${paths}${!segments.length ? '<text class="chart-empty" x="80" y="33" text-anchor="middle">等待采样</text>' : ""}</svg><div class="chart-axis"><span>−${short(minutes)}</span><span>−${short(minutes / 2)}</span><span>现在</span></div></div>`;
}

function resourceCard(gpu: Gpu, stale: boolean, samples: Sample[], minutes: number, cpu = false, updatedAt?: number): string {
  const low = !cpu && !stale && isAvailable(gpu);
  const utilization = gpu.utilization;
  const age = updatedAt ? Math.max(0, Math.floor(Date.now() / 1000 - updatedAt)) : null;
  return `<article class="gpu-card ${stale ? "stale" : ""}">
    <div class="gpu-heading"><span class="gpu-index">${cpu ? "CPU" : gpu.index.toString().padStart(2, "0")}</span><div class="gpu-name"><h3>${esc(gpu.name.replace(/^NVIDIA /, ""))}</h3><span>${stale ? "上次数据 · 已过期" : `最近 ${historyLabel(minutes)}${cpu || gpu.temperature === null ? "" : ` · ${gpu.temperature}°C`}`}</span></div><span class="load-label ${low ? "low" : ""}">${stale ? "已断线" : low ? "低负载" : utilization === null ? "未知" : cpu ? "运行中" : "使用中"}</span></div>
    <div class="metrics"><div class="metric"><div class="metric-title"><span>${cpu ? "CPU" : "GPU"} 利用率</span><strong>${utilization === null ? "—" : `${utilization}<small>%</small>`}</strong></div>${lineChart(samples, "utilization", 100, "100%", minutes)}</div><div class="metric"><div class="metric-title"><span>${cpu ? "内存" : "显存"}</span><strong>${gib(gpu.memoryUsed)}<small> / ${gib(gpu.memoryTotal)} GiB</small></strong></div>${lineChart(samples, "memoryUsed", gpu.memoryTotal ?? 0, `${gib(gpu.memoryTotal)} GiB`, minutes)}</div></div>
    <div class="sample-age">${age === null ? "等待采样" : age < 2 ? "刚刚更新" : `${age} 秒前更新`}</div>
  </article>`;
}

function serverSection(server: Server): string {
  const local = server.source === "local";
  const monitor = effectiveMonitor(snapshot, server.id);
  const gpuOnline = server.status === "online";
  const cpuOnline = Boolean(server.cpu && !server.cpuError);
  const online = (monitor.gpuEnabled && gpuOnline) || (monitor.cpuEnabled && cpuOnline);
  const gpus = monitor.gpuEnabled ? server.gpus.filter(g => !availableOnly || (gpuOnline && isAvailable(g))) : [];
  const gpuCards = gpus.map(g => resourceCard(g, !gpuOnline, server.history?.[g.uuid] ?? [], effectivePolicy(snapshot, server.id, g.uuid).historyMinutes, false, server.gpuUpdatedAt?.[g.uuid] ?? server.updatedAt ?? undefined)).join("");
  const cpuCard = monitor.cpuEnabled && !availableOnly ? server.cpu ? resourceCard({ index: 0, uuid: "CPU", name: `CPU · ${server.cpu.logicalCpus} 逻辑处理器`, utilization: server.cpu.utilization, memoryUsed: server.cpu.memoryUsed, memoryTotal: server.cpu.memoryTotal, temperature: null }, !cpuOnline, server.history?.CPU ?? [], monitor.cpuHistoryMinutes, true, server.cpu.updatedAt) : '<div class="connection-message">正在读取 CPU 与系统内存…</div>' : "";
  return `<section class="server-section"><div class="server-heading"><div class="server-title">${icon(local ? "chip" : "server")}<h2 title="${esc(server.target)}">${esc(server.name)}</h2><span class="status-dot ${online ? "online" : monitor.cpuEnabled || monitor.gpuEnabled ? "error" : ""}"></span></div><details class="server-menu" data-server-menu="${esc(server.id)}"><summary aria-label="服务器选项">${icon("more")}</summary><div><button data-reconnect="${esc(server.id)}">${local ? "重新检测" : "重新连接"}</button><button data-remove="${esc(server.id)}" class="danger">${removeId === server.id ? "再次点击确认移除" : local ? "移除本机" : "移除服务器"}</button></div></details></div>
    ${setupId === server.id ? '<div class="connection-message">正在接入… 如需密码，将在 App 内提示。</div>' : ""}
    ${monitor.gpuEnabled && server.error && setupId !== server.id ? `<div class="connection-error"><span>${local ? "本机 GPU 不可用" : "GPU 连接未就绪"}</span><p>${esc(server.error)}</p><button class="text-button" data-reconnect="${esc(server.id)}">${local ? "重新检测" : "重新连接"} ${icon("arrow")}</button></div>` : ""}
    ${monitor.cpuEnabled && server.cpuError ? `<div class="connection-error"><span>CPU 采集未就绪</span><p>${esc(server.cpuError)}</p></div>` : ""}
    ${cpuCard}${gpuCards}
    ${!monitor.cpuEnabled && !monitor.gpuEnabled ? `<div class="connection-message">监控已暂停，可在设置中开启 ${local ? "GPU" : "CPU 或 GPU"}。</div>` : ""}
    ${monitor.gpuEnabled && !gpus.length && gpuOnline ? '<div class="empty-filter">暂无符合条件的 GPU</div>' : monitor.gpuEnabled && !gpus.length && !server.error && setupId !== server.id ? '<div class="connection-message">正在读取 GPU 数据…</div>' : ""}
  </section>`;
}

function render() {
  const activeControl = document.activeElement;
  const focusedServer = activeControl?.closest<HTMLDetailsElement>("[data-server-menu]")?.dataset.serverMenu;
  const focusedSelector = activeControl?.matches("summary") ? "summary"
    : activeControl?.hasAttribute("data-reconnect") ? "[data-reconnect]"
    : activeControl?.hasAttribute("data-remove") ? "[data-remove]" : null;
  const openedMenus = new Set(Array.from($("#servers").querySelectorAll<HTMLDetailsElement>("details[open]"), menu => menu.dataset.serverMenu));
  const counts = summary(snapshot);
  $("#stats").innerHTML = `<div><strong>${counts.total}<small>张</small></strong><span>在线 GPU</span></div><div><strong class="accent">${counts.available}<small>张</small></strong><span title="利用率与显存占用均低于 10%；不代表已预留或保证任务可运行">低负载</span></div><div><strong>${counts.online}<small> / ${snapshot.servers.length}</small></strong><span>在线设备</span></div>`;
  $("#live").innerHTML = `<i class="${counts.online ? "" : "inactive"}"></i>${demo ? "演示数据" : counts.online ? "实时" : "未连接"}`;
  $("#server-count").textContent = `${snapshot.servers.length} 台设备`;
  $("#refresh").classList.toggle("spinning", snapshot.refreshing);
  $("#servers").innerHTML = snapshot.servers.length ? snapshot.servers.map(serverSection).join("") : `<section class="empty"><div class="empty-chip">${icon("chip")}</div><h2>让每一张 GPU 都看得见</h2><p>添加本机或 SSH 服务器，随时查看<br>GPU 利用率与显存占用。</p><button class="primary" id="empty-add">${icon("plus")} 添加设备</button><span class="empty-note">本机 / SSH · 自动刷新</span></section>`;
  $("#servers").querySelectorAll<HTMLDetailsElement>("[data-server-menu]").forEach(menu => {
    menu.open = openedMenus.has(menu.dataset.serverMenu);
    if (menu.dataset.serverMenu === focusedServer && focusedSelector) {
      menu.querySelector<HTMLElement>(focusedSelector)?.focus({ preventScroll: true });
    }
  });
  updateOverlayControls();
  if (snapshot.historyWarning && snapshot.historyWarning !== lastHistoryWarning) notify(snapshot.historyWarning);
  lastHistoryWarning = snapshot.historyWarning ?? null;
}

function setAddSource(source: "local" | "ssh") {
  addSource = source;
  const local = source === "local";
  const fields = $<HTMLFieldSetElement>("#ssh-fields");
  fields.hidden = local;
  fields.disabled = local;
  $("#add-description").textContent = local ? "查看本机 NVIDIA GPU，无需 SSH 或密码。" : "填写连接信息，即可查看服务器 GPU 状态。";
  $("#connect-button").innerHTML = local ? `添加本机 ${icon("plus")}` : `连接服务器 ${icon("arrow")}`;
  $("#form-error").hidden = true;
  document.querySelectorAll<HTMLButtonElement>("[data-add-source]").forEach(button => {
    button.setAttribute("aria-pressed", String(button.dataset.addSource === source));
  });
  if (local) $<HTMLInputElement>("#server-password").value = "";
  else void inspectKey();
}

function openAdd() {
  if (!desktop) notify("请在桌面 App 中添加设备。");
  setAddSource(addSource);
  $<HTMLDialogElement>("#add-dialog").showModal();
  (addSource === "local" ? $("#connect-button") : $("#host")).focus();
}

async function connect(id: string, password: string | null = null) {
  setupId = id;
  render();
  try { await invoke("setup_server", { id, password }); notify("免密连接已验证，正在读取监控数据。"); }
  catch (error) { notify(String(error)); }
  finally { password = null; setupId = null; render(); }
}

function showPrompt(next: typeof prompt) {
  prompt = next;
  const dialog = $<HTMLDialogElement>("#auth-dialog");
  $<HTMLInputElement>("#password").value = "";
  if (!next) { dialog.close(); return; }
  $("#auth-title").textContent = next.confirmation ? "确认服务器身份" : "输入 SSH 密码";
  $("#auth-description").textContent = next.confirmation ? "首次连接，请核对服务器指纹后继续。" : "请输入下面提示所对应的密码或密钥口令。输错后可以重试。";
  $("#auth-prompt").textContent = next.text;
  $("#password-label").hidden = next.confirmation;
  $<HTMLInputElement>("#password").required = !next.confirmation;
  $("#submit-auth").textContent = next.confirmation ? "信任并继续" : "继续连接";
  if (!dialog.open) dialog.showModal();
  if (!next.confirmation) $<HTMLInputElement>("#password").focus();
}

async function answer(cancel = false) {
  if (!prompt) return;
  const id = prompt.id;
  const field = $<HTMLInputElement>("#password");
  const value = cancel ? null : prompt.confirmation ? "yes" : field.value;
  field.value = "";
  $<HTMLDialogElement>("#auth-dialog").close();
  try { await invoke("answer_prompt", { id, value }); } catch (error) { notify(String(error)); }
}

$("#add").onclick = openAdd;
$("#add-dialog").addEventListener("click", event => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-add-source]");
  if (button && !button.disabled) setAddSource(button.dataset.addSource as "local" | "ssh");
});
$("#close-add").onclick = () => $<HTMLDialogElement>("#add-dialog").close();
$("#add-dialog").addEventListener("close", () => { $<HTMLInputElement>("#server-password").value = ""; });
$("#hide").onclick = () => { if (desktop) void getCurrentWindow().hide(); };
$("#refresh").onclick = () => { if (desktop) void invoke("refresh_now").catch(e => notify(String(e))); else notify("演示预览不连接真实服务器。"); };
$("#titlebar").onmousedown = event => { if (desktop && event.button === 0 && !(event.target as HTMLElement).closest("button")) void getCurrentWindow().startDragging(); };
$("#all-filter").onclick = () => setFilter(false);
$("#available-filter").onclick = () => setFilter(true);
$("#load-help").onclick = () => $<HTMLDialogElement>("#load-dialog").showModal();
$("#close-load").onclick = () => $<HTMLDialogElement>("#load-dialog").close();
function setFilter(value: boolean) {
  availableOnly = value;
  for (const [selector, active] of [["#all-filter", !value], ["#available-filter", value]] as const) { $(selector).classList.toggle("active", active); $(selector).setAttribute("aria-pressed", String(active)); }
  render();
}

$("#servers").onclick = event => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button");
  if (!button) return;
  if (button.id === "empty-add") openAdd();
  if (button.dataset.reconnect && desktop) {
    const server = snapshot.servers.find(server => server.id === button.dataset.reconnect);
    if (server?.source === "local") void invoke("refresh_now").catch(error => notify(String(error)));
    else if (server) { if (setupId) notify("请先完成当前连接。"); else void connect(server.id); }
  }
  if (button.dataset.remove && desktop) {
    const id = button.dataset.remove;
    if (setupId === id) { notify("请先完成或取消当前认证。"); return; }
    if (removeId !== id) { removeId = id; button.textContent = "再次点击确认移除"; return; }
    removeId = null;
    void invoke("remove_server", { id }).catch(e => notify(String(e)));
  }
};

function setSettingsOpen(open: boolean) {
  if (open) renderSettings();
  $("#dashboard").hidden = open;
  $("#settings-page").hidden = !open;
  $("#settings").setAttribute("aria-pressed", String(open));
  $("#settings").title = open ? "返回主页面" : "设置";
  if (!open) render();
}

function renderSettings(preserveEdits = true) {
  const content = $("#settings-content");
  const edited = preserveEdits ? Array.from(content.querySelectorAll<HTMLFormElement>('form[data-dirty="true"], form[data-saving="true"]')) : [];
  const opened = Array.from(content.querySelectorAll<HTMLDetailsElement>('details[open]')).map(details => details.dataset.settingsServer);
  const panels = new Map(Array.from(content.querySelectorAll<HTMLElement>('[data-resource-scope]')).map(group => [group.dataset.resourceScope, group.querySelector<HTMLButtonElement>('[data-resource-tab][aria-selected="true"]')?.dataset.resourceTab ?? "gpu"]));
  content.innerHTML = settingsContent(snapshot);
  // Keep other unsaved forms intact when a global save updates inherited values.
  for (const form of edited) {
    const replacement = Array.from(content.querySelectorAll<HTMLFormElement>("form")).find(next => next.dataset.settingsKey === form.dataset.settingsKey);
    replacement?.replaceWith(form);
  }
  content.querySelectorAll<HTMLDetailsElement>("details").forEach(details => { details.open = opened.includes(details.dataset.settingsServer); });
  content.querySelectorAll<HTMLElement>('[data-resource-scope]').forEach(group => { setResourcePanel(group, panels.get(group.dataset.resourceScope) ?? "gpu"); });
  updateOverlayControls();
}

function updateOverlayControls() {
  const enabled = document.querySelector<HTMLInputElement>('form[data-kind="overlay"] input[name="enabled"]');
  if (enabled && enabled.form?.dataset.enabledDirty !== "true") enabled.checked = effectiveOverlay(snapshot).enabled;
  const editing = Boolean(snapshot.overlayEditing);
  document.querySelectorAll<HTMLButtonElement>("[data-overlay-edit]").forEach(button => {
    button.textContent = editing ? "完成调整，恢复穿透" : "调整宽度";
    button.disabled = overlayControlBusy || (!effectiveOverlay(snapshot).enabled && !editing);
  });
  document.querySelectorAll<HTMLElement>("[data-overlay-mode]").forEach(label => {
    label.textContent = editing ? "调整中 · 暂停自动隐藏" : effectiveOverlay(snapshot).enabled ? "鼠标移入时隐藏" : "浮窗已关闭";
  });
}

function setResourcePanel(group: HTMLElement, selected: string, focus = false) {
  group.querySelectorAll<HTMLButtonElement>("[data-resource-tab]").forEach(button => {
    const active = button.dataset.resourceTab === selected;
    button.setAttribute("aria-selected", String(active));
    button.tabIndex = active ? 0 : -1;
    if (active && focus) button.focus();
  });
  group.querySelectorAll<HTMLElement>("[data-resource-panel]").forEach(panel => { panel.hidden = panel.dataset.resourcePanel !== selected; });
}

function lockForm(form: HTMLFormElement): () => void {
  const controls = Array.from(form.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLButtonElement>("input, select, button"),
    control => ({ control, disabled: control.disabled }));
  form.dataset.saving = "true";
  controls.forEach(({ control }) => { control.disabled = true; });
  return () => {
    delete form.dataset.saving;
    controls.forEach(({ control, disabled }) => { control.disabled = disabled; });
  };
}

async function settingsSaved(form: HTMLFormElement, message: string) {
  if (desktop) snapshot = await invoke<Snapshot>("get_snapshot");
  delete form.dataset.dirty;
  delete form.dataset.enabledDirty;
  delete form.dataset.saving;
  renderSettings();
  render();
  notify(desktop ? [message, snapshot.historyWarning].filter(Boolean).join("\n") : "已应用到当前演示预览。");
}
$("#settings").onclick = () => setSettingsOpen($("#settings-page").hidden);
$("#back-settings").onclick = () => setSettingsOpen(false);
$("#settings-content").addEventListener("input", event => {
  const target = event.target as HTMLInputElement;
  const form = target.closest("form");
  if (form) form.dataset.dirty = "true";
  if (form?.dataset.kind === "overlay" && target.name === "enabled") form.dataset.enabledDirty = "true";
  if (target.type === "range") {
    const output = form?.querySelector<HTMLOutputElement>(`output[data-range-output="${target.name}"]`);
    if (output) output.value = `${target.value}%`;
  }
});
$("#settings-content").addEventListener("keydown", event => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-resource-tab]");
  if (!button || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const selected = event.key === "Home" ? "gpu" : event.key === "End" ? "cpu" : button.dataset.resourceTab === "gpu" ? "cpu" : "gpu";
  setResourcePanel(button.closest<HTMLElement>("[data-resource-scope]")!, selected, true);
});
$("#settings-content").addEventListener("change", event => {
  const target = event.target as HTMLSelectElement;
  const form = target.closest("form");
  if (form) form.dataset.dirty = "true";
  if (form?.dataset.kind === "overlay" && target.name === "enabled") form.dataset.enabledDirty = "true";
  if (target.name !== "mode") return;
  if (!form) return;
  for (const [selector, hidden] of [["[data-adaptive]", target.value === "fixed"], ["[data-fixed]", target.value === "adaptive"]] as const) {
    const area = form.querySelector<HTMLElement>(selector)!;
    area.hidden = hidden;
    area.querySelectorAll<HTMLInputElement>("input").forEach(input => { input.disabled = hidden; });
  }
});
$("#settings-content").addEventListener("submit", async event => {
  event.preventDefault();
  if (settingsSaving) return;
  settingsSaving = true;
  const form = event.target as HTMLFormElement;
  const serverId = form.dataset.server!;
  const field = (name: string) => form.elements.namedItem(name) as HTMLInputElement;
  const unlock = lockForm(form);
  try {
    if (form.dataset.kind === "overlay") {
      const draft: OverlaySettings = { enabled: field("enabled").checked, opacity: Number(field("opacity").value), backgroundOpacity: Number(field("backgroundOpacity").value), fontSize: Number(field("fontSize").value), serverId: field("serverId").value || null };
      const settings = applyOverlayEdits(effectiveOverlay(snapshot), draft, form.dataset.enabledDirty === "true");
      if (!Number.isInteger(settings.opacity) || settings.opacity < 30 || settings.opacity > 100 || !Number.isInteger(settings.backgroundOpacity) || settings.backgroundOpacity < 0 || settings.backgroundOpacity > 80 || !Number.isInteger(settings.fontSize) || settings.fontSize < 12 || settings.fontSize > 28) throw new Error("请检查浮窗透明度与字号范围。");
      if (settings.serverId && !snapshot.servers.some(server => server.id === settings.serverId)) throw new Error("请选择当前设备或全部设备。");
      if (desktop) await invoke("set_overlay_settings", { settings });
      else { snapshot.overlay = settings; if (!settings.enabled) snapshot.overlayEditing = false; }
    } else if (["monitor", "global-monitor", "cpu", "global-cpu"].includes(form.dataset.kind!)) {
      const section: MonitorSection = form.dataset.kind!.endsWith("cpu") ? "cpu" : "switches";
      const values: Partial<MonitorPolicy> = section === "cpu"
        ? { cpuSeconds: Number(field("cpuSeconds").value), cpuHistoryMinutes: Number(field("cpuHistoryMinutes").value) }
        : { cpuEnabled: field("cpuEnabled")?.checked ?? false, gpuEnabled: field("gpuEnabled").checked };
      if (form.dataset.kind!.startsWith("global-")) {
        const global = globalSettings(snapshot);
        const settings = { ...global, monitor: { ...global.monitor, ...values } };
        if (desktop) await invoke("set_global_settings", { settings });
        else snapshot.global = settings;
      } else {
        if (desktop) await invoke(section === "cpu" ? "set_cpu_settings" : "set_monitor_switches", { serverId, ...values });
        else updateMonitorOverride(snapshot, serverId, section, values);
      }
    } else {
      const gpuUuid = form.dataset.gpu!;
      const draft: RefreshPolicy = { mode: field("mode").value as "adaptive" | "fixed", fastSeconds: Number(field("fastSeconds").value), normalSeconds: Number(field("normalSeconds").value), slowSeconds: Number(field("slowSeconds").value), fixedSeconds: Number(field("fixedSeconds").value), historyMinutes: Number(field("historyMinutes").value) };
      const saved = form.dataset.kind === "global-gpu" ? globalSettings(snapshot).gpu : effectivePolicy(snapshot, serverId, gpuUuid);
      const policy = applyPolicyEdits(saved, draft);
      if (policy.fastSeconds > policy.normalSeconds || policy.normalSeconds > policy.slowSeconds) throw new Error("低利用率的间隔不能大于中、高利用率的间隔。");
      if (form.dataset.kind === "global-gpu") {
        const settings = { ...globalSettings(snapshot), gpu: policy };
        if (desktop) await invoke("set_global_settings", { settings });
        else snapshot.global = settings;
      } else {
        if (desktop) await invoke("set_refresh_policy", { serverId, gpuUuid, policy });
        else { snapshot.policies ??= {}; snapshot.policies[`${serverId}/${gpuUuid}`] = policy; }
      }
    }
    await settingsSaved(form, form.dataset.kind === "overlay" ? "浮窗设置已保存。" : form.dataset.kind?.startsWith("global-") ? "全局设置已保存，跟随全局的项已更新。" : "单独设置已保存，优先于全局默认。");
  } catch (error) { notify(String(error)); }
  finally { unlock(); settingsSaving = false; }
});
$("#settings-content").addEventListener("click", async event => {
  const overlayButton = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-overlay-edit]");
  if (overlayButton) {
    if (overlayControlBusy || settingsSaving) return;
    overlayControlBusy = true;
    updateOverlayControls();
    try {
      const editing = !snapshot.overlayEditing;
      if (desktop) { await invoke(editing ? "begin_overlay_edit" : "end_overlay_edit"); snapshot = await invoke<Snapshot>("get_snapshot"); }
      else snapshot.overlayEditing = editing;
      notify(desktop ? editing ? "已暂停自动隐藏；边缘调整宽度，高度随内容自动变化。完成后请恢复穿透。" : "已恢复自动隐藏与鼠标穿透。" : "演示已切换调整状态，桌面浮窗请在 App 中使用。");
    } catch (error) { notify(String(error)); }
    finally { overlayControlBusy = false; updateOverlayControls(); }
    return;
  }
  const tab = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-resource-tab]");
  if (tab) { setResourcePanel(tab.closest<HTMLElement>("[data-resource-scope]")!, tab.dataset.resourceTab!); return; }
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-reset]");
  if (!button || settingsSaving) return;
  const form = button.closest("form")!;
  const serverId = form.dataset.server!;
  settingsSaving = true;
  const unlock = lockForm(form);
  try {
    if (form.dataset.kind === "monitor" || form.dataset.kind === "cpu") {
      const section: MonitorSection = form.dataset.kind === "cpu" ? "cpu" : "switches";
      if (desktop) await invoke("reset_monitor_section", { serverId, section });
      else updateMonitorOverride(snapshot, serverId, section, null);
    } else {
      const gpuUuid = form.dataset.gpu!;
      if (desktop) await invoke("reset_refresh_policy", { serverId, gpuUuid });
      else delete snapshot.policies?.[`${serverId}/${gpuUuid}`];
    }
    await settingsSaved(form, "已恢复跟随全局，今后会随全局设置更新。");
  } catch (error) { notify(String(error)); }
  finally { unlock(); settingsSaving = false; }
});

async function inspectKey() {
  if (!desktop || addSource !== "ssh") return;
  try {
    const key = await invoke<{ path: string; exists: boolean }>("inspect_key", { sshCommand: "ssh localhost" });
    if (addSource !== "ssh" || !$<HTMLDialogElement>("#add-dialog").open) return;
    $("#key-info").innerHTML = `${icon(key.exists ? "check" : "key")}<span>${key.exists ? "已检测到本机密钥，将直接复用。" : "未发现本机密钥，连接时自动创建。"}<small>${esc(key.path)}</small></span>`;
  } catch (error) { if (addSource === "ssh") $("#key-info").textContent = String(error); }
}

$("#add-form").onsubmit = async event => {
  event.preventDefault();
  if (!desktop) { notify("请在桌面 App 中添加设备。"); return; }
  if (setupId && addSource === "ssh") { notify("请先完成当前连接。"); return; }
  const form = $<HTMLFormElement>("#add-form");
  if (form.dataset.saving === "true") return;
  const unlock = lockForm(form);
  try {
    if (addSource === "local") {
      await invoke<string>("add_local");
      snapshot = await invoke<Snapshot>("get_snapshot");
      $<HTMLDialogElement>("#add-dialog").close();
      form.reset();
      render();
      if (!$("#settings-page").hidden) renderSettings();
      notify("本机已加入，正在检测 GPU。");
      return;
    }
    const id = await invoke<string>("add_connection", { host: $<HTMLInputElement>("#host").value, port: Number($<HTMLInputElement>("#port").value), username: $<HTMLInputElement>("#username").value });
    const password = $<HTMLInputElement>("#server-password").value;
    $<HTMLInputElement>("#server-password").value = "";
    $<HTMLDialogElement>("#add-dialog").close();
    form.reset();
    await connect(id, password || null);
  } catch (error) { $("#form-error").textContent = String(error); $("#form-error").hidden = false; }
  finally { unlock(); }
};
$("#auth-form").onsubmit = event => { event.preventDefault(); void answer(); };
$("#cancel-auth").onclick = () => { void answer(true); };
$("#auth-dialog").addEventListener("cancel", event => { event.preventDefault(); void answer(true); });

async function initialize() {
  render();
  if (desktop) {
    await listen<Snapshot>("snapshot", event => { snapshot = event.payload; render(); });
    await listen<typeof prompt>("auth-prompt", event => showPrompt(event.payload));
    snapshot = await invoke<Snapshot>("get_snapshot");
    showPrompt(await invoke("get_auth_prompt"));
  } else if (demo) {
    const gpu = (index: number, name: string, utilization: number, memoryUsed: number, memoryTotal: number, temperature: number): Gpu => ({ index, uuid: `demo-${index}`, name, utilization, memoryUsed, memoryTotal, temperature });
    snapshot.servers = [
      { id: "demo-1", name: "训练服务器", target: "demo-training", sshCommand: "", status: "online", error: null, updatedAt: Date.now() / 1000, gpus: [gpu(0, "NVIDIA RTX 4090", 96, 21504, 24576, 68), gpu(1, "NVIDIA RTX 4090", 2, 384, 24576, 34)] },
      { id: "demo-2", name: "推理服务器", target: "demo-inference", sshCommand: "", status: "online", error: null, updatedAt: Date.now() / 1000, gpus: [gpu(0, "NVIDIA A100 80GB", 42, 47104, 81920, 49)] },
    ];
    for (const server of snapshot.servers) {
      server.history = {};
      for (const gpu of server.gpus) {
        server.history[gpu.uuid] = Array.from({ length: 241 }, (_, i) => ({
          at: Math.floor(Date.now() / 1000) - 3600 + i * 15,
          utilization: Math.max(0, Math.min(100, gpu.utilization! + Math.sin(i / 11 + gpu.index) * (gpu.utilization! < 10 ? 2 : 14) + Math.sin(i / 3) * 3)),
          memoryUsed: Math.max(0, Math.min(gpu.memoryTotal!, gpu.memoryUsed! * (0.92 + 0.07 * Math.sin(i / 26)))),
          memoryTotal: gpu.memoryTotal,
        }));
      }
    }
  }
  render();
}
void initialize().catch(error => notify(String(error)));

// Advance chart time and sample ages without querying SSH between scheduled polls.
setInterval(() => {
  if (document.hidden || $("#dashboard").hidden) return;
  if (!desktop) { render(); return; }
  void getCurrentWindow().isVisible().then(visible => {
    if (visible && !document.hidden && !$("#dashboard").hidden) render();
  }).catch(() => {}); // The window may have closed while visibility was being queried.
}, 5000);
