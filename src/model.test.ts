import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { applyOverlayEdits, applyPolicyEdits, chartSegments, defaultMonitor, defaultOverlaySettings, defaultPolicy, effectiveMonitor, effectivePolicy, globalSettings, hasMonitorOverride, updateMonitorOverride, escapeHtml, isAvailable, summary, type Gpu, type Sample, type Snapshot } from "./model.ts";

const gpu: Gpu = { index: 0, uuid: "GPU-1", name: "RTX 4090", utilization: 2, memoryUsed: 512, memoryTotal: 24576, temperature: 35 };

describe("independent settings edits", () => {
  it("saves fixed mode without applying hidden adaptive edits", () => {
    const saved = { ...defaultPolicy(), fastSeconds: 3, normalSeconds: 10, slowSeconds: 20 };
    const result = applyPolicyEdits(saved, { ...saved, mode: "fixed", fixedSeconds: 300, historyMinutes: 180, fastSeconds: 100, normalSeconds: 0, slowSeconds: NaN });
    assert.deepStrictEqual(result, { ...saved, mode: "fixed", fixedSeconds: 300, historyMinutes: 180 });
    assert.strictEqual(saved.mode, "adaptive");
  });
  it("saves adaptive mode without applying a cleared hidden fixed interval", () => {
    const saved = { ...defaultPolicy(), mode: "fixed" as const, fixedSeconds: 120 };
    const result = applyPolicyEdits(saved, { ...saved, mode: "adaptive", fastSeconds: 2, normalSeconds: 5, slowSeconds: 15, fixedSeconds: 0 });
    assert.deepStrictEqual(result, { ...saved, mode: "adaptive", fastSeconds: 2, normalSeconds: 5, slowSeconds: 15 });
  });
  it("keeps appearance edits while accepting a tray toggle of the untouched checkbox", () => {
    const saved = { ...defaultOverlaySettings(), enabled: false };
    const draft = { ...defaultOverlaySettings(), opacity: 55, backgroundOpacity: 20, fontSize: 24, serverId: "one" };
    assert.deepStrictEqual(applyOverlayEdits(saved, draft, false), { ...draft, enabled: false });
    assert.strictEqual(draft.enabled, true);
  });
  it("preserves an explicitly edited checkbox across opposite tray updates", () => {
    const saved = defaultOverlaySettings();
    assert.strictEqual(applyOverlayEdits(saved, { ...saved, enabled: false }, true).enabled, false);
    assert.strictEqual(applyOverlayEdits({ ...saved, enabled: false }, saved, true).enabled, true);
  });
});

describe("global defaults and independent overrides", () => {
  const empty = (): Snapshot => ({ interval: 15, refreshing: false, servers: [] });
  it("local devices ignore inherited and overridden CPU monitoring without changing GPU settings", () => {
    const snapshot = empty();
    snapshot.servers = [{ id: "local", source: "local", name: "本机", sshCommand: "", target: "本机", status: "online", error: null, updatedAt: 1, gpus: [gpu], cpu: { logicalCpus: 8, utilization: 1, memoryUsed: 1, memoryTotal: 2, updatedAt: 1 } }];
    snapshot.global = { gpu: { ...defaultPolicy(), historyMinutes: 180 }, monitor: { ...defaultMonitor(), gpuEnabled: false, cpuEnabled: true } };
    assert.strictEqual(effectiveMonitor(snapshot, "local").cpuEnabled, false);
    assert.deepStrictEqual(summary(snapshot), { online: 0, total: 0, available: 0 });
    snapshot.monitors = { local: { cpuEnabled: true, gpuEnabled: true } };
    assert.deepStrictEqual(effectiveMonitor(snapshot, "local"), { ...defaultMonitor(), gpuEnabled: true, cpuEnabled: false });
    assert.strictEqual(effectivePolicy(snapshot, "local", gpu.uuid).historyMinutes, 180);
    assert.deepStrictEqual(summary(snapshot), { online: 1, total: 1, available: 1 });
    assert.strictEqual(snapshot.global.monitor.cpuEnabled, true);
    assert.strictEqual(snapshot.monitors.local.cpuEnabled, true);
  });
  it("local source handling leaves legacy and explicit SSH devices unchanged", () => {
    const snapshot = empty();
    snapshot.servers = [{ id: "remote", name: "remote", sshCommand: "ssh host", target: "host", status: "online", error: null, updatedAt: 1, gpus: [gpu] }];
    snapshot.global = { gpu: defaultPolicy(), monitor: { ...defaultMonitor(), cpuEnabled: true } };
    const legacy = effectiveMonitor(snapshot, "remote");
    assert.strictEqual(legacy.cpuEnabled, true);
    snapshot.servers[0].source = "ssh";
    assert.deepStrictEqual(effectiveMonitor(snapshot, "remote"), legacy);
  });
  it("saves and resets CPU settings without changing inherited monitoring switches", () => {
    const snapshot = empty();
    snapshot.global = { gpu: defaultPolicy(), monitor: { ...defaultMonitor(), gpuEnabled: false, cpuEnabled: true } };
    updateMonitorOverride(snapshot, "server", "cpu", { cpuSeconds: 8, cpuHistoryMinutes: 180 });
    assert.deepStrictEqual(effectiveMonitor(snapshot, "server"), { cpuEnabled: true, gpuEnabled: false, cpuSeconds: 8, cpuHistoryMinutes: 180 });
    assert.strictEqual(hasMonitorOverride(snapshot, "server", "switches"), false);
    snapshot.global.monitor.cpuEnabled = false;
    assert.strictEqual(effectiveMonitor(snapshot, "server").cpuEnabled, false);
    updateMonitorOverride(snapshot, "server", "switches", { cpuEnabled: true, gpuEnabled: true });
    assert.strictEqual(effectiveMonitor(snapshot, "server").cpuSeconds, 8);
    updateMonitorOverride(snapshot, "server", "cpu", null);
    assert.deepStrictEqual(effectiveMonitor(snapshot, "server"), { ...defaultMonitor(), cpuEnabled: true });
    assert.strictEqual(hasMonitorOverride(snapshot, "server", "cpu"), false);
    assert.strictEqual(hasMonitorOverride(snapshot, "server", "switches"), true);
    updateMonitorOverride(snapshot, "server", "switches", null);
    assert.strictEqual(snapshot.monitors?.server, undefined);
    assert.deepStrictEqual(effectiveMonitor(snapshot, "server"), snapshot.global.monitor);
  });
  it("preserves old snapshots and applies global defaults to new resources", () => {
    const snapshot = empty();
    assert.deepStrictEqual(globalSettings(snapshot), { gpu: defaultPolicy(), monitor: defaultMonitor() });
    snapshot.global = { gpu: { ...defaultPolicy(), historyMinutes: 180, fastSeconds: 3 }, monitor: { ...defaultMonitor(), cpuEnabled: true } };
    assert.strictEqual(effectivePolicy(snapshot, "new-server", "new-gpu").historyMinutes, 180);
    assert.strictEqual(effectiveMonitor(snapshot, "new-server").cpuEnabled, true);
  });
  it("keeps per-GPU overrides across global changes and inherits again after reset", () => {
    const snapshot = empty();
    const own = { ...defaultPolicy(), fixedSeconds: 9, historyMinutes: 30 };
    snapshot.policies = { "server/GPU-1": own };
    snapshot.global = { gpu: { ...defaultPolicy(), fixedSeconds: 60, historyMinutes: 360 }, monitor: defaultMonitor() };
    assert.deepStrictEqual(effectivePolicy(snapshot, "server", "GPU-1"), own);
    assert.strictEqual(effectivePolicy(snapshot, "server", "GPU-2").historyMinutes, 360);
    delete snapshot.policies["server/GPU-1"];
    assert.strictEqual(effectivePolicy(snapshot, "server", "GPU-1").fixedSeconds, 60);
    snapshot.global.gpu = { ...snapshot.global.gpu, historyMinutes: 1440 };
    assert.strictEqual(effectivePolicy(snapshot, "server", "GPU-1").historyMinutes, 1440);
  });
  it("uses inherited monitoring switches in online counts without overriding server choices", () => {
    const snapshot = empty();
    snapshot.servers = [{ id: "server", name: "server", sshCommand: "ssh host", target: "host", status: "online", error: null, updatedAt: 1, gpus: [gpu], cpu: { logicalCpus: 8, utilization: 1, memoryUsed: 1, memoryTotal: 2, updatedAt: 1 } }];
    snapshot.global = { gpu: defaultPolicy(), monitor: { ...defaultMonitor(), gpuEnabled: false, cpuEnabled: true } };
    assert.deepStrictEqual(summary(snapshot), { online: 1, total: 0, available: 0 });
    snapshot.monitors = { server: defaultMonitor() };
    assert.deepStrictEqual(summary(snapshot), { online: 1, total: 1, available: 1 });
    delete snapshot.monitors.server;
    snapshot.global.monitor.cpuEnabled = false;
    assert.deepStrictEqual(summary(snapshot), { online: 0, total: 0, available: 0 });
  });
});

describe("live capacity", () => {
  it("does not call a memory-occupied or unknown GPU available", () => {
    assert.strictEqual(isAvailable(gpu), true);
    assert.strictEqual(isAvailable({ ...gpu, memoryUsed: 20000 }), false);
    assert.strictEqual(isAvailable({ ...gpu, utilization: null }), false);
    assert.strictEqual(isAvailable({ ...gpu, memoryTotal: 0 }), false);
  });
  it("excludes stale data after disconnection from totals", () => {
    assert.deepStrictEqual(summary({ interval: 15, refreshing: false, servers: [{ id: "one", name: "one", sshCommand: "ssh host", target: "host", status: "error", error: "timeout", updatedAt: 1, gpus: [gpu] }] }), { online: 0, total: 0, available: 0 });
  });
  it("escapes remote text and command attributes", () => {
    assert.strictEqual(escapeHtml('<img onerror="x">&'), "&lt;img onerror=&quot;x&quot;&gt;&amp;");
  });
});

describe("history charts", () => {
  it("uses a fixed one-hour axis and omits samples outside the window", () => {
    const sample = (at: number, utilization: number | null): Sample => ({ at, utilization, memoryUsed: 0, memoryTotal: 1024 });
    const lines = chartSegments([sample(1, 10), sample(3600, 50), sample(7200, 100), sample(7201, 1)], "utilization", 100, 7200, 15);
    assert.deepStrictEqual(lines, [[[0, 29]], [[160, 6]]]);
  });
  it("breaks on failed polls and does not invent zero usage", () => {
    const samples = [100, null, 80].map((value, index) => ({ at: 1000 + index * 15, utilization: value, memoryUsed: null, memoryTotal: 1024 }));
    assert.deepStrictEqual(chartSegments(samples, "utilization", 100, 1030, 15).map(s => s.length), [1, 1]);
    assert.deepStrictEqual(chartSegments(samples, "memoryUsed", 1024, 1030, 15), []);
  });
  it("uses the selected window and respects deliberately slow sampling", () => {
    const samples = [0, 1800, 2100, 2400].map(at => ({ at, utilization: 50, memoryUsed: 1, memoryTotal: 2, intervalSeconds: 300 }));
    assert.deepStrictEqual(chartSegments(samples, "utilization", 100, 2400, 15, 900), [[[53.33, 29], [106.67, 29], [160, 29]]]);
    assert.deepStrictEqual(chartSegments(samples, "utilization", 100, 2400, 15, 10800).map(s => s.length), [1, 3]);
  });
});
