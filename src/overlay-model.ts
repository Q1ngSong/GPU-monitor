import { effectiveMonitor, effectivePolicy, gib, type Snapshot } from "./model.ts";

export interface OverlayMetric {
  key: string;
  label: string;
  utilization: number | null;
  memory: string | null;
  state: "live" | "offline" | "stale" | "waiting";
}

export interface OverlayRow {
  key: string;
  name: string;
  metrics: OverlayMetric[];
}

/** Allow the configured interval plus one bounded SSH query before marking a value stale. */
function stateOf(online: boolean, updatedAt: number | null | undefined, deadline: number, now: number): OverlayMetric["state"] {
  if (!online) return "offline";
  if (!updatedAt) return "waiting";
  return now > deadline + 30 ? "stale" : "live";
}

export function overlayRows(snapshot: Snapshot, serverId: string | null, now: number): OverlayRow[] {
  return snapshot.servers.filter(server => !serverId || server.id === serverId).flatMap(server => {
    const monitor = effectiveMonitor(snapshot, server.id);
    if (!monitor.gpuEnabled && !monitor.cpuEnabled) return [];
    const metrics: OverlayMetric[] = [];
    if (monitor.gpuEnabled) {
      for (const gpu of server.gpus) {
        const policy = effectivePolicy(snapshot, server.id, gpu.uuid);
        const interval = policy.mode === "fixed" ? policy.fixedSeconds : gpu.utilization !== null && gpu.utilization < 10 ? policy.fastSeconds : gpu.utilization !== null && gpu.utilization < 60 ? policy.normalSeconds : policy.slowSeconds;
        const at = server.gpuUpdatedAt?.[gpu.uuid] ?? server.updatedAt;
        const due = server.nextPoll?.[gpu.uuid] ?? (at ?? now) + interval;
        const state = stateOf(server.status === "online" && !server.error, at, due, now);
        metrics.push({
          key: gpu.uuid, label: `GPU ${gpu.index}`, state,
          utilization: state === "live" && Number.isFinite(gpu.utilization) ? gpu.utilization : null,
          memory: state === "live" && gpu.memoryUsed !== null && gpu.memoryTotal !== null && gpu.memoryTotal > 0 ? `${gib(gpu.memoryUsed)}/${gib(gpu.memoryTotal)}` : null,
        });
      }
      if (!server.gpus.length) metrics.push({ key: "GPU", label: "GPU", utilization: null, memory: null, state: server.error ? "offline" : "waiting" });
    }
    if (monitor.cpuEnabled) {
      const cpu = server.cpu;
      const state = server.cpuError ? "offline" : !cpu ? "waiting" : stateOf(true, cpu.updatedAt, server.cpuNextPoll || cpu.updatedAt + monitor.cpuSeconds, now);
      metrics.push({ key: "CPU", label: "CPU", utilization: state === "live" && cpu && Number.isFinite(cpu.utilization) ? cpu.utilization : null, memory: null, state });
    }
    return [{ key: server.id, name: server.name, metrics }];
  });
}
