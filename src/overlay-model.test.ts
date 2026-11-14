import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { defaultMonitor, defaultPolicy, type Snapshot } from "./model.ts";
import { overlayRows } from "./overlay-model.ts";

function snapshot(): Snapshot {
  return { interval: 15, refreshing: false, servers: [{ id: "one", name: "Training", target: "host", sshCommand: "ssh host", status: "online", error: null, updatedAt: 100,
    gpus: [{ index: 0, uuid: "GPU-one", name: "RTX", utilization: 10, memoryUsed: 1024, memoryTotal: 2048, temperature: 30 }],
    cpu: { logicalCpus: 8, utilization: 6, memoryUsed: 4096, memoryTotal: 8192, updatedAt: 100 },
  }] };
}

describe("overlay values", () => {
  it("local overlays retain GPU values while ignoring inherited CPU monitoring", () => {
    const state = snapshot();
    state.servers[0].source = "local";
    state.global = { gpu: defaultPolicy(), monitor: { ...defaultMonitor(), cpuEnabled: true } };
    state.monitors = { one: { cpuEnabled: true } };
    const rows = overlayRows(state, "one", 101);
    assert.strictEqual(rows.length, 1);
    assert.deepStrictEqual(rows[0].metrics.map(metric => metric.label), ["GPU 0"]);
    assert.partialDeepStrictEqual(rows[0].metrics[0], { state: "live", utilization: 10, memory: "1.0/2.0" });
  });
  it("respects selected servers and independently enabled CPU/GPU monitors", () => {
    const state = snapshot();
    assert.deepStrictEqual(overlayRows(state, "missing", 101), []);
    state.global = { gpu: defaultPolicy(), monitor: { ...defaultMonitor(), gpuEnabled: false, cpuEnabled: true } };
    assert.deepStrictEqual(overlayRows(state, null, 101)[0].metrics.map(metric => metric.label), ["CPU"]);
    state.monitors = { one: { gpuEnabled: true, cpuEnabled: false } };
    assert.deepStrictEqual(overlayRows(state, null, 101)[0].metrics.map(metric => metric.label), ["GPU 0"]);
    state.monitors.one.gpuEnabled = false;
    assert.deepStrictEqual(overlayRows(state, null, 101), []);
  });
  it("never presents disconnected or overdue cached readings as live usage", () => {
    const state = snapshot();
    state.servers[0].status = "error";
    assert.partialDeepStrictEqual(overlayRows(state, null, 101)[0].metrics[0], { state: "offline", utilization: null, memory: null });
    state.servers[0].status = "online";
    assert.partialDeepStrictEqual(overlayRows(state, null, 146)[0].metrics[0], { state: "stale", utilization: null, memory: null });
    state.policies = { "one/GPU-one": { ...defaultPolicy(), mode: "fixed", fixedSeconds: 300 } };
    assert.partialDeepStrictEqual(overlayRows(state, null, 146)[0].metrics[0], { state: "live", utilization: 10, memory: "1.0/2.0" });
  });
  it("keeps unknown utilization unknown while retaining a valid memory measurement", () => {
    const state = snapshot();
    state.servers[0].gpus[0].utilization = null;
    assert.partialDeepStrictEqual(overlayRows(state, null, 101)[0].metrics[0], { state: "live", utilization: null, memory: "1.0/2.0" });
  });
});
