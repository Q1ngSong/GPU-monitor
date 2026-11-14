import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { defaultMonitor, defaultOverlaySettings, defaultPolicy, effectiveOverlay, type Snapshot } from "./model.ts";
import { settingsContent } from "./settings.ts";

describe("settings page", () => {
  it("local settings expose GPU controls without CPU switches or panels", () => {
    const snapshot: Snapshot = {
      interval: 15, refreshing: false,
      global: { gpu: defaultPolicy(), monitor: { ...defaultMonitor(), cpuEnabled: true } },
      servers: [{ id: "local", source: "local", name: "本机", target: "本机", sshCommand: "", status: "online", error: null, updatedAt: 1,
        gpus: [{ index: 0, uuid: "GPU-local", name: "NVIDIA RTX", utilization: 0, memoryUsed: 0, memoryTotal: 1, temperature: null }] },
        { id: "remote", name: "remote", target: "host", sshCommand: "ssh host", status: "online", error: null, updatedAt: 1, gpus: [] }],
    };
    const html = settingsContent(snapshot);
    const local = html.match(/<details[^>]*data-settings-server="local"[\s\S]*?<\/details>/)![0];
    const remote = html.match(/<details[^>]*data-settings-server="remote"[\s\S]*?<\/details>/)![0];
    assert.ok(local.includes('name="gpuEnabled"'));
    assert.ok(local.includes('data-kind="gpu"'));
    assert.ok(local.includes('name="historyMinutes"'));
    assert.ok(!local.includes('name="cpuEnabled"'));
    assert.ok(!local.includes('data-kind="cpu"'));
    assert.ok(!local.includes('data-resource-panel="cpu"'));
    assert.ok(!local.includes('data-resource-scope'));
    assert.ok(remote.includes('name="cpuEnabled"'));
    assert.ok(remote.includes('data-kind="cpu"'));
    assert.ok(html.includes('data-kind="global-cpu"'));
  });
  it("keeps overlay controls independent from monitoring with transparent defaults for old snapshots", () => {
    const snapshot: Snapshot = { servers: [], interval: 15, refreshing: false };
    assert.deepStrictEqual(effectiveOverlay(snapshot), { enabled: true, opacity: 100, backgroundOpacity: 0, fontSize: 16, serverId: null });
    const html = settingsContent(snapshot);
    const form = html.match(/<form[^>]*data-kind="overlay"[\s\S]*?<\/form>/)![0];
    assert.ok((form).includes('name="backgroundOpacity"'));
    assert.ok((form).includes('value="" selected>全部设备'));
    assert.ok(!(form).includes('name="cpuSeconds"'));
    assert.ok(!(form).includes('name="gpuEnabled"'));
    assert.ok(html.indexOf('data-kind="overlay"') < html.indexOf('data-kind="global-monitor"'));
    assert.ok((html).includes("点击与滚轮直接作用于下方页面"));
    assert.ok((html).includes("鼠标移入面板时，内容隐藏并出现可拖动的小柄"));
    assert.ok((html).includes("移出面板范围后，内容恢复、小柄隐藏"));
    assert.ok((html).includes("鼠标移入时隐藏"));
  });
  it("retains the selected server and a way to restore click-through while editing", () => {
    const snapshot: Snapshot = { interval: 15, refreshing: false, servers: [{ id: "server", name: "<training>", target: "host", sshCommand: "ssh host", status: "online", error: null, updatedAt: 1, gpus: [] }], overlay: { ...defaultOverlaySettings(), enabled: false, serverId: "server", opacity: 75 }, overlayEditing: true };
    const html = settingsContent(snapshot);
    assert.strictEqual(effectiveOverlay(snapshot).opacity, 75);
    assert.ok((html).includes('<option value="server" selected>&lt;training&gt;</option>'));
    assert.ok((html).includes('data-overlay-edit >完成调整，恢复穿透'));
    snapshot.overlayEditing = false;
    assert.ok((settingsContent(snapshot)).includes('data-overlay-edit disabled>调整宽度'));
  });
  it("offers global settings before the first server is connected", () => {
    const html = settingsContent({ servers: [], interval: 15, refreshing: false });
    assert.ok((html).includes('data-kind="global-monitor"'));
    assert.ok((html).includes('data-kind="global-gpu"'));
    assert.ok((html).includes('data-kind="global-cpu"'));
    assert.ok((html).includes("保存监控内容"));
    assert.ok((html).includes('role="tablist"'));
    assert.ok((html).includes("CPU 设置"));
    assert.ok(!(html).includes("监控内容与 CPU"));
    const switchesForm = html.match(/<form[^>]*data-kind="global-monitor"[\s\S]*?<\/form>/)![0];
    const cpuForm = html.match(/<form[^>]*data-kind="global-cpu"[\s\S]*?<\/form>/)![0];
    assert.ok((switchesForm).includes('name="gpuEnabled"'));
    assert.ok(!(switchesForm).includes('name="cpuSeconds"'));
    assert.ok((cpuForm).includes('name="cpuSeconds"'));
    assert.ok(!(cpuForm).includes('name="cpuEnabled"'));
    assert.ok(!(html).includes("data-reset"));
  });
  it("makes override ownership explicit and offers reset only for independent settings", () => {
    const snapshot: Snapshot = {
      interval: 15, refreshing: false,
      servers: [{ id: "server", name: "<training>", target: "host", sshCommand: "ssh host", status: "online", error: null, updatedAt: 1,
        gpus: [0, 1].map(index => ({ index, uuid: `GPU-${index}`, name: "NVIDIA RTX", utilization: 0, memoryUsed: 0, memoryTotal: 1, temperature: null })) }],
      global: { gpu: { ...defaultPolicy(), historyMinutes: 180 }, monitor: defaultMonitor() },
      policies: { "server/GPU-0": { ...defaultPolicy(), historyMinutes: 30 } },
    };
    const html = settingsContent(snapshot);
    assert.ok((html).includes("&lt;training&gt;"));
    assert.strictEqual((html.match(/data-reset/g))?.length, 1);
    assert.ok((html).includes("恢复跟随全局"));
    assert.strictEqual((html.match(/settings-source overridden/g))?.length, 1);
    assert.strictEqual((html.match(/>跟随全局<\/span>/g))?.length, 3);
    assert.ok((html).includes('<option value="30" selected>30 分钟</option>'));
    assert.ok((html).includes('<option value="180" selected>3 小时</option>'));
  });
  it("labels CPU-only overrides independently from inherited monitoring switches", () => {
    const snapshot: Snapshot = { interval: 15, refreshing: false, servers: [{ id: "server", name: "server", target: "host", sshCommand: "ssh host", status: "online", error: null, updatedAt: 1, gpus: [] }], monitors: { server: { cpuSeconds: 8 } } };
    const html = settingsContent(snapshot);
    const switchesForm = html.match(/<form[^>]*data-kind="monitor"[\s\S]*?<\/form>/)![0];
    const cpuForm = html.match(/<form[^>]*data-kind="cpu"[\s\S]*?<\/form>/)![0];
    assert.ok((switchesForm).includes("跟随全局"));
    assert.ok(!(switchesForm).includes("data-reset"));
    assert.ok((cpuForm).includes("settings-source overridden"));
    assert.ok((cpuForm).includes("data-reset"));
  });
});
