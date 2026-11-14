export interface Gpu {
  index: number;
  uuid: string;
  name: string;
  utilization: number | null;
  memoryUsed: number | null;
  memoryTotal: number | null;
  temperature: number | null;
}

export interface Sample {
  at: number;
  utilization: number | null;
  memoryUsed: number | null;
  memoryTotal: number | null;
  intervalSeconds?: number;
}

export interface RefreshPolicy { mode: "adaptive" | "fixed"; fastSeconds: number; normalSeconds: number; slowSeconds: number; fixedSeconds: number; historyMinutes: number }
export interface MonitorPolicy { cpuEnabled: boolean; gpuEnabled: boolean; cpuSeconds: number; cpuHistoryMinutes: number }
export type MonitorSection = "switches" | "cpu";
export interface GlobalSettings { gpu: RefreshPolicy; monitor: MonitorPolicy }
export interface OverlaySettings { enabled: boolean; opacity: number; backgroundOpacity: number; fontSize: number; serverId: string | null }
export const defaultOverlaySettings = (): OverlaySettings => ({ enabled: true, opacity: 100, backgroundOpacity: 0, fontSize: 16, serverId: null });
export const effectiveOverlay = (snapshot: Snapshot): OverlaySettings => snapshot.overlay ?? defaultOverlaySettings();
export const defaultPolicy = (): RefreshPolicy => ({ mode: "adaptive", fastSeconds: 5, normalSeconds: 15, slowSeconds: 30, fixedSeconds: 15, historyMinutes: 60 });
export const defaultMonitor = (): MonitorPolicy => ({ cpuEnabled: false, gpuEnabled: true, cpuSeconds: 15, cpuHistoryMinutes: 60 });
export const globalSettings = (snapshot: Snapshot): GlobalSettings => snapshot.global ?? { gpu: defaultPolicy(), monitor: defaultMonitor() };
export const effectivePolicy = (snapshot: Snapshot, serverId: string, gpuUuid: string): RefreshPolicy => snapshot.policies?.[`${serverId}/${gpuUuid}`] ?? globalSettings(snapshot).gpu;
export function effectiveMonitor(snapshot: Snapshot, serverId: string): MonitorPolicy {
  const monitor = { ...globalSettings(snapshot).monitor, ...snapshot.monitors?.[serverId] };
  if (snapshot.servers.some(server => server.id === serverId && server.source === "local")) monitor.cpuEnabled = false;
  return monitor;
}

/** Hidden fields keep their saved values when the refresh mode changes. */
export function applyPolicyEdits(saved: RefreshPolicy, draft: RefreshPolicy): RefreshPolicy {
  const visible = draft.mode === "fixed" ? { fixedSeconds: draft.fixedSeconds }
    : { fastSeconds: draft.fastSeconds, normalSeconds: draft.normalSeconds, slowSeconds: draft.slowSeconds };
  return { ...saved, mode: draft.mode, historyMinutes: draft.historyMinutes, ...visible };
}

/** A tray toggle wins unless the user explicitly edited the enabled checkbox. */
export function applyOverlayEdits(saved: OverlaySettings, draft: OverlaySettings, enabledEdited: boolean): OverlaySettings {
  return { ...draft, enabled: enabledEdited ? draft.enabled : saved.enabled };
}

const monitorSectionFields = { switches: ["cpuEnabled", "gpuEnabled"], cpu: ["cpuSeconds", "cpuHistoryMinutes"] } as const;
export function hasMonitorOverride(snapshot: Snapshot, serverId: string, section: MonitorSection): boolean {
  return monitorSectionFields[section].some(key => snapshot.monitors?.[serverId]?.[key] !== undefined);
}

/** Updating one settings panel must preserve the other panel's values and inheritance. */
export function updateMonitorOverride(snapshot: Snapshot, serverId: string, section: MonitorSection, values: Partial<MonitorPolicy> | null) {
  const override = { ...snapshot.monitors?.[serverId] };
  for (const key of monitorSectionFields[section]) delete override[key];
  const selected = values === null ? {} : Object.fromEntries(monitorSectionFields[section].filter(key => values[key] !== undefined).map(key => [key, values[key]]));
  Object.assign(override, selected);
  snapshot.monitors ??= {};
  if (Object.keys(override).length) snapshot.monitors[serverId] = override;
  else delete snapshot.monitors[serverId];
}

export interface Server {
  id: string;
  source?: "local" | "ssh";
  name: string;
  sshCommand: string;
  target: string;
  gpus: Gpu[];
  status: "connecting" | "online" | "error" | "paused";
  error: string | null;
  updatedAt: number | null;
  history?: Record<string, Sample[]>;
  gpuUpdatedAt?: Record<string, number>;
  nextPoll?: Record<string, number>;
  cpu?: { logicalCpus: number; utilization: number | null; memoryUsed: number; memoryTotal: number; updatedAt: number } | null;
  cpuError?: string | null;
  cpuNextPoll?: number;
}

export interface Snapshot {
  servers: Server[];
  interval: number;
  refreshing: boolean;
  historyWarning?: string | null;
  policies?: Record<string, RefreshPolicy>;
  monitors?: Record<string, Partial<MonitorPolicy>>;
  global?: GlobalSettings;
  overlay?: OverlaySettings;
  overlayEditing?: boolean;
}

/** Split at unavailable samples or missed polls; never bridge an offline period. */
export function chartSegments(samples: Sample[], metric: "utilization" | "memoryUsed", maximum: number, now: number, interval: number, windowSeconds = 3600): Array<Array<[number, number]>> {
  const segments: Array<Array<[number, number]>> = [];
  let segment: Array<[number, number]> = [];
  let previous: number | null = null;
  let previousInterval = interval;
  for (const sample of samples) {
    if (sample.at < now - windowSeconds || sample.at > now) continue;
    const value = sample[metric];
    if (value === null || !Number.isFinite(value) || maximum <= 0) {
      if (segment.length) segments.push(segment);
      segment = []; previous = null; continue;
    }
    if (previous !== null && sample.at - previous > Math.max(60, previousInterval * 3)) {
      if (segment.length) segments.push(segment);
      segment = [];
    }
    const x = (sample.at - (now - windowSeconds)) / windowSeconds * 160;
    const y = 52 - Math.min(1, Math.max(0, value / maximum)) * 46;
    segment.push([Number(x.toFixed(2)), Number(y.toFixed(2))]);
    previous = sample.at;
    previousInterval = sample.intervalSeconds ?? interval;
  }
  if (segment.length) segments.push(segment);
  return segments;
}

/** A low-load hint requires both live utilization and memory measurements. */
export function isAvailable(gpu: Gpu): boolean {
  return gpu.utilization !== null && gpu.utilization < 10 &&
    gpu.memoryUsed !== null && gpu.memoryTotal !== null && gpu.memoryTotal > 0 &&
    gpu.memoryUsed / gpu.memoryTotal < 0.1;
}

export function gib(mib: number | null): string {
  return mib === null ? "—" : (mib / 1024).toFixed(1);
}

/** Escape all server-provided strings before using them in HTML templates. */
export function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

export function summary(snapshot: Snapshot) {
  const online = snapshot.servers.filter(s => {
    const monitor = effectiveMonitor(snapshot, s.id);
    return (monitor.gpuEnabled && s.status === "online") || (monitor.cpuEnabled && s.cpu && !s.cpuError);
  });
  const gpus = online.filter(s => s.status === "online" && effectiveMonitor(snapshot, s.id).gpuEnabled).flatMap(s => s.gpus);
  return { online: online.length, total: gpus.length, available: gpus.filter(isAvailable).length };
}
