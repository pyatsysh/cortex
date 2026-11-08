// Names Cortex does not control (process names, mount points, the driver string) must reach the
// board as text, never as markup: a process called "<!--" once blanked the Top Processes card.
// The test runs the page's own script from src/index.html in a sandbox with a minimal DOM and a
// fake Tauri backend, then reads the HTML the board wrote.  Run: node --test tests/board.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const page = readFileSync(new URL("../src/index.html", import.meta.url), "utf8");
const script = page.match(/<script>([\s\S]*?)<\/script>/)[1];

class El {
  constructor(tag = "div") {
    Object.assign(this, { tag, html: "", kids: {}, style: {}, dataset: {}, hidden: false, value: "", textContent: "", title: "" });
    this.classList = { add() {}, remove() {}, toggle() {}, contains: () => false };
  }
  set innerHTML(v) { this.html = String(v); this.kids = {}; this.writes = (this.writes || 0) + 1; }
  get innerHTML() { return this.html; }
  querySelector(sel) { return (this.kids[sel] ??= new El()); }
  querySelectorAll() { return []; }
  addEventListener(type, fn) { (this.handlers ??= {})[type] = fn; }
  setAttribute(k, v) { this[k] = v; }
  appendChild(c) { return c; }
  remove() {}
  closest() { return null; }
  getContext() { return new Proxy({}, { get: () => () => {} }); }
  get clientWidth() { return 0; }
}

class Canvas extends El {
  constructor() {
    super("canvas");
    this.calls = [];
    this.ctx = new Proxy({}, { get: (_, k) => (...args) => this.calls.push([k, ...args]), set: () => true });
  }
  getContext() { return this.ctx; }
  get clientWidth() { return 290; }
  get clientHeight() { return 120; }
}

async function renderBoard(metrics, sysinfo, options = {}) {
  const created = [];
  const byId = {};
  const modes = ["dash", "detail", "info", "training"].map(view => Object.assign(new El(), { dataset: { view } }));
  const document = {
    querySelector: sel => (byId[sel] ??= sel.startsWith("#chart-") ? new Canvas() : new El()),
    getElementById: id => (byId["#" + id] ??= new El()),
    querySelectorAll: sel => (sel === ".mode" ? modes : []),
    createElement: tag => { const e = new El(tag); created.push(e); return e; },
    addEventListener(type, fn) { (this.handlers ??= {})[type] = fn; },
    documentElement: new El(),
    activeElement: null,
  };
  const store = {};
  let poll, frames = 0, metricCalls = 0;
  const sandbox = {
    document,
    matchMedia: () => ({ matches: false }),
    localStorage: { getItem: k => store[k] ?? null, setItem: (k, v) => { store[k] = String(v); } },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
    requestAnimationFrame() { frames++; },
    setInterval: fn => { poll = fn; return 0; },
    clearInterval() {},
    setTimeout: () => 0,
    performance: { now: () => 0 },
    navigator: {},
    devicePixelRatio: 1,
    addEventListener() {},
    __TAURI__: {
      core: { invoke: cmd => { if (cmd === "get_metrics") { metricCalls++; return options.getMetrics ? options.getMetrics() : Promise.resolve(metrics); } return Promise.resolve(sysinfo); } },
      window: { getCurrentWindow: () => ({ setAlwaysOnTop() {} }) },
    },
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(script, sandbox);
  for (let i = 0; i < 5; i++) await new Promise(r => setImmediate(r));
  const card = key => created.find(e => e.dataset.key === key);
  return {
    get procs() { return card("procs").kids[".ptb"].innerHTML; },
    get drives() { return card("storage").kids[".drives"].innerHTML; },
    get gpuLine() { return card("gpu0")?.kids[".gl"].innerHTML; },
    card, poll: () => poll(), document,
    get frames() { return frames; },
    get metricCalls() { return metricCalls; },
    showView: view => modes.find(m => m.dataset.view === view).handlers.click(),
    chartCalls: sel => (byId[sel] ??= new Canvas()).calls,
  };
}

const hostile = ["<!--", '<img src=x onerror="alert(1)">', '"><b>x</b>', "a&b <i>c</i>", "kworker/0:1"];
const metrics = {
  gpus: [{ name: "GPU", util: 5, temp: 40, power: 30, mem_util: 1, vram_pct: 4, vram_used: 0.9, vram_total: 24, clock_core: 210, clock_mem: 405 }],
  cpu: { util: 10, temp: 50 },
  ram: { util: 25, used: 8, total: 31.2 },
  disks: hostile.map((name, i) => ({ name: "/mnt/" + name, free: 10 + i, total: 100 })),
  net: { up: 0.1, down: 0.2 },
  procs: hostile.map((name, i) => ({ name, cpu: 50 - i, gpu: null, ram: 10 + i, color: "#88C0D0" })),
  cores: [{ util: 10, freq: 3700 }],
  interval_ms: 1000,
};
const sysinfo = { os: "Linux", kernel: "6.8", hostname: "h", cpu: "c", cpu_threads: 1, cpu_mhz: 3700, gpu: "GPU",
  gpu_driver: "595.91.07<script>", cuda: "13.2", ram_total: 31.2, uptime: 1, arch: "x86_64", wsl: false };

const text = s => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

test("hostile process names stay text in Top Processes", async () => {
  const { procs } = await renderBoard(metrics, sysinfo);
  assert.equal((procs.match(/<tr>/g) || []).length, 5, "all five rows survive");
  for (const name of hostile) assert.ok(procs.includes(">" + text(name) + "<"), "name shown as text: " + name);
  assert.ok(!procs.includes("<!--") && !procs.includes("<img") && !procs.includes("<b>"), "no markup from names");
  assert.ok(procs.includes('title="' + text('"><b>x</b>') + '"'), "quotes cannot leave the title attribute");
});

test("hostile mount points stay text in Storage", async () => {
  const { drives } = await renderBoard(metrics, sysinfo);
  assert.equal((drives.match(/class="drive"/g) || []).length, hostile.length, "every mount keeps its row");
  for (const name of hostile) assert.ok(drives.includes(text("/mnt/" + name)), "mount shown as text: " + name);
  assert.ok(!drives.includes("<!--") && !drives.includes("<img") && !drives.includes("<i>"), "no markup from mounts");
});

test("the driver string stays text on the GPU card", async () => {
  const { gpuLine } = await renderBoard(metrics, sysinfo);
  assert.ok(gpuLine.includes("595.91.07&lt;script&gt;") && !gpuLine.includes("<script"), gpuLine);
});

// The Detailed view's GPU chart once drew a flat 0 % line when NVML was unavailable.
const moveTos = calls => calls.filter(c => c[0] === "moveTo").length;  // the grid draws four

test("an unavailable GPU is labelled on the Detailed chart, never drawn as zero", async () => {
  const board = await renderBoard({ ...metrics, gpus: [] }, sysinfo);
  board.showView("detail");
  const calls = board.chartCalls("#chart-gpu");
  assert.equal(moveTos(calls), 4, "only the grid: no engine or memory line");
  assert.ok(calls.some(c => c[0] === "fillText" && c[1] === "N/A"), "the chart says N/A");
});

test("an available GPU still draws both lines on the Detailed chart", async () => {
  const board = await renderBoard(metrics, sysinfo);
  board.showView("detail");
  const calls = board.chartCalls("#chart-gpu");
  assert.equal(moveTos(calls), 6, "grid plus the engine and memory lines");
  assert.ok(!calls.some(c => c[0] === "fillText"), "no N/A label");
});

test("unchanged readings retain their DOM and changed readings appear without animation", async () => {
  let current = metrics;
  const board = await renderBoard(metrics, sysinfo, { getMetrics: () => Promise.resolve(current) });
  const cpu = board.card("cpu").kids[".val"];
  const procs = board.card("procs").kids[".ptb"];
  const storage = board.card("storage").kids[".drives"];
  const writes = [cpu.writes, procs.writes, storage.writes];
  await board.poll();
  assert.deepEqual([cpu.writes, procs.writes, storage.writes], writes);
  current = { ...metrics, cpu: { ...metrics.cpu, util: 67.1 }, net: { up: 16.8, down: 4.2 } };
  await board.poll();
  assert.equal(cpu.innerHTML, "67<small>%</small>");
  assert.equal(board.card("network").kids[".nu"].innerHTML, "16.8");
  assert.equal(cpu.writes, writes[0] + 1);
  assert.equal(board.frames, 0, "no intermediate numeric frames");
});

test("slow and failed samples cannot accumulate concurrent requests", async () => {
  let resolve, reject;
  const board = await renderBoard(metrics, sysinfo, { getMetrics: () => new Promise((yes, no) => { resolve = yes; reject = no; }) });
  assert.equal(board.metricCalls, 1);
  await board.poll(); await board.poll();
  assert.equal(board.metricCalls, 1, "one request remains in flight");
  resolve(metrics);
  await new Promise(r => setImmediate(r));
  const failed = board.poll();
  assert.equal(board.metricCalls, 2);
  reject(new Error("synthetic unavailable backend"));
  await failed;
  const recovered = board.poll();
  assert.equal(board.metricCalls, 3, "failure releases the sampling guard");
  resolve(metrics); await recovered;
});

test("hidden windows stop sampling and refresh when visible", async () => {
  const board = await renderBoard(metrics, sysinfo);
  board.document.hidden = true;
  await board.poll(); await board.poll();
  assert.equal(board.metricCalls, 1);
  board.document.hidden = false;
  board.document.handlers.visibilitychange();
  await new Promise(r => setImmediate(r));
  assert.equal(board.metricCalls, 2);
});

test("an inactive dashboard stays untouched and receives the latest sample on return", async () => {
  let current = metrics;
  const board = await renderBoard(metrics, sysinfo, { getMetrics: () => Promise.resolve(current) });
  const cpu = board.card("cpu").kids[".val"], before = cpu.writes;
  board.showView("info");
  current = { ...metrics, cpu: { ...metrics.cpu, util: 81 } };
  await board.poll();
  assert.equal(cpu.writes, before);
  board.showView("dash");
  assert.equal(cpu.innerHTML, "81<small>%</small>");
});

// The R1 review, 16 October 2026 (C1): sampling pauses while the window is hidden, so the samples of that interval
// are missing. Every chart must show the break, rather than join the samples either side of it into a line that
// hides a stall.
test("a hidden interval is a gap in every chart, never a continuous line", async () => {
  const board = await renderBoard(metrics, sysinfo);
  await board.poll();
  board.document.hidden = true;
  await board.poll(); await board.poll();
  board.document.hidden = false;
  board.document.handlers.visibilitychange();
  for (let i = 0; i < 3; i++) await new Promise(r => setImmediate(r));
  board.showView("detail");
  assert.equal(moveTos(board.chartCalls("#chart-cpu")), 6, "the grid, then one line broken once");
  assert.equal(moveTos(board.chartCalls("#chart-ram")), 6, "the grid, then one line broken once");
  assert.equal(moveTos(board.chartCalls("#chart-gpu")), 8, "engine and memory lines, each broken once");
  assert.equal(moveTos(board.chartCalls("#chart-net")), 8, "up and down lines, each broken once");
});

// C2: a get_metrics call that never answers holds the sampling guard. The status must not go on saying Live.
test("a sample that never returns makes the status stale until one does", async () => {
  let answer;
  const board = await renderBoard(metrics, sysinfo, { getMetrics: () => new Promise(yes => { answer = yes; }) });
  const dot = board.document.querySelector("#statusDot");
  assert.equal(dot.title, "Live");
  await board.poll(); await board.poll();
  assert.equal(dot.title, "Live", "two intervals are not yet stale");
  await board.poll();
  assert.match(dot.title, /^Stale/);
  assert.equal(board.metricCalls, 1, "still one request in flight");
  answer(metrics);
  for (let i = 0; i < 3; i++) await new Promise(r => setImmediate(r));
  assert.equal(dot.title, "Live", "a completed sample restores Live");
});
