import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { defaultMonitor, effectiveOverlay, escapeHtml, type Snapshot } from "./model";
import { overlayRows, type OverlayMetric } from "./overlay-model";
import "./overlay.css";

const desktop = isTauri();
const params = new URLSearchParams(location.search);
const demo = !desktop && params.has("demo");
const shell = document.querySelector<HTMLDivElement>("#overlay-shell")!;
const readout = document.querySelector<HTMLDivElement>("#readout")!;
const toolbar = document.querySelector<HTMLDivElement>("#edit-toolbar")!;
const error = document.querySelector<HTMLDivElement>("#overlay-error")!;
let snapshot: Snapshot = { servers: [], interval: 15, refreshing: false };
let content = "";
let pendingHeight: number | null = null;
let requestedHeight = 0;
let resizing = false;

// Measure natural content, independent of the current native window height.
function fitContent() {
  if (!desktop) return;
  const height = Math.min(16384, Math.max(44, Math.ceil(shell.getBoundingClientRect().height)));
  if (height === requestedHeight && (resizing || Math.abs(window.innerHeight - height) <= 1)) return;
  pendingHeight = height;
  if (!resizing) void resizeToContent();
}

async function resizeToContent() {
  resizing = true;
  try {
    while (pendingHeight !== null) {
      const height = pendingHeight;
      pendingHeight = null;
      requestedHeight = height;
      await invoke("resize_overlay_content", { height });
    }
  } catch (reason) {
    pendingHeight = null;
    showError(reason);
  } finally {
    resizing = false;
  }
}

const stateLabels = { live: "", offline: "已断线", stale: "已过期", waiting: "等待采样" };

function metricHtml(metric: OverlayMetric): string {
  const unavailable = metric.state !== "live";
  const number = metric.utilization === null ? "—" : `${Math.round(metric.utilization)}<small>%</small>`;
  return `<span class="resource ${unavailable ? "unavailable" : ""}" title="${stateLabels[metric.state] || (metric.utilization === null ? "利用率未知" : "当前采样")}"><span class="resource-label">${metric.label}</span><strong>${number}</strong>${metric.memory !== null ? `<span class="memory" title="${metric.unifiedMemory ? "GPU 使用 / 系统总量，CPU 与 GPU 共享" : "已用 / 总显存"}"><span>${metric.unifiedMemory ? "统一内存" : "显存"}</span> ${metric.memory}<small> GiB</small></span>` : ""}${unavailable ? `<small class="state-label">${stateLabels[metric.state]}</small>` : ""}</span>`;
}

function render() {
  const options = effectiveOverlay(snapshot);
  const editing = Boolean(snapshot.overlayEditing);
  document.documentElement.style.setProperty("--text-opacity", String(options.opacity / 100));
  document.documentElement.style.setProperty("--background-opacity", String(options.backgroundOpacity / 100));
  document.documentElement.style.setProperty("--metric-size", `${options.fontSize}px`);
  document.body.classList.toggle("editing", editing);
  toolbar.hidden = !editing;
  const rows = overlayRows(snapshot, options.serverId, Date.now() / 1000);
  const empty = !desktop && !demo ? "请启动桌面 App" : snapshot.servers.length ? "监控已暂停或未选择服务器" : "等待连接服务器";
  const next = rows.length ? rows.map(row => `<section class="server-row"><span class="server-name" title="${escapeHtml(row.name)}">${escapeHtml(row.name)}</span><span class="server-metrics">${row.metrics.map(metricHtml).join("")}</span></section>`).join("") : `<div class="empty-readout">${empty}</div>`;
  if (content !== next) {
    readout.innerHTML = next;
    content = next;
  }
  fitContent();
}

function showError(reason: unknown) {
  error.textContent = String(reason);
  error.hidden = false;
}

document.querySelector<HTMLSpanElement>("#drag-handle")!.addEventListener("mousedown", event => {
  if (desktop && snapshot.overlayEditing && event.button === 0) void getCurrentWindow().startDragging().catch(showError);
});
document.querySelector<HTMLButtonElement>("#finish-edit")!.onclick = async () => {
  error.hidden = true;
  try {
    if (desktop) await invoke("end_overlay_edit");
    else { snapshot.overlayEditing = false; render(); }
  } catch (reason) { showError(reason); }
};
document.querySelector<HTMLButtonElement>("#open-details")!.onclick = () => {
  if (desktop) void invoke("open_main_window").catch(showError);
};
document.addEventListener("keydown", event => {
  if (event.key === "Escape" && snapshot.overlayEditing) document.querySelector<HTMLButtonElement>("#finish-edit")!.click();
});

new ResizeObserver(fitContent).observe(shell);

function demoSnapshot(): Snapshot {
  const now = Math.floor(Date.now() / 1000);
  return {
    interval: 15, refreshing: false, overlayEditing: params.has("edit"),
    monitors: { "demo-training": { ...defaultMonitor(), cpuEnabled: true } },
    servers: [
      { id: "demo-training", name: "训练服务器", target: "demo", sshCommand: "", status: "online", error: null, updatedAt: now, gpus: [
        { index: 0, uuid: "demo-0", name: "RTX 5090", utilization: 10, memoryUsed: 8192, memoryTotal: 32607, temperature: 50 },
        { index: 1, uuid: "demo-1", name: "RTX 5090", utilization: 92, memoryUsed: 24600, memoryTotal: 32607, temperature: 68 },
      ], cpu: { logicalCpus: 32, utilization: 6, memoryUsed: 32768, memoryTotal: 131072, updatedAt: now } },
      { id: "demo-inference", name: "推理服务器", target: "demo", sshCommand: "", status: "online", error: null, updatedAt: now, gpus: [
        { index: 0, uuid: "demo-2", name: "L40", utilization: 76, memoryUsed: 19180, memoryTotal: 46068, temperature: 61 },
      ] },
    ],
  };
}

async function initialize() {
  if (desktop) {
    await listen<Snapshot>("snapshot", event => { snapshot = event.payload; render(); });
    snapshot = await invoke<Snapshot>("get_snapshot");
  } else if (demo) {
    snapshot = demoSnapshot();
    document.querySelector<HTMLDivElement>("#demo-label")!.hidden = false;
  }
  render();
  setInterval(() => {
    if (demo) {
      const now = Math.floor(Date.now() / 1000);
      for (const server of snapshot.servers) { server.updatedAt = now; if (server.cpu) server.cpu.updatedAt = now; }
    }
    render();
  }, 1000);
}
void initialize().catch(showError);
