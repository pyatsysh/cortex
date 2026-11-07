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
  set innerHTML(v) { this.html = String(v); this.kids = {}; }
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

async function renderBoard(metrics, sysinfo) {
  const created = [];
  const byId = {};
  const modes = ["dash", "detail", "info", "training"].map(view => Object.assign(new El(), { dataset: { view } }));
  const document = {
    querySelector: sel => (byId[sel] ??= sel.startsWith("#chart-") ? new Canvas() : new El()),
    getElementById: id => (byId["#" + id] ??= new El()),
    querySelectorAll: sel => (sel === ".mode" ? modes : []),
    createElement: tag => { const e = new El(tag); created.push(e); return e; },
    addEventListener() {},
    documentElement: new El(),
    activeElement: null,
  };
  const store = {};
  const sandbox = {
    document,
    matchMedia: () => ({ matches: false }),
    localStorage: { getItem: k => store[k] ?? null, setItem: (k, v) => { store[k] = String(v); } },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
    requestAnimationFrame() {},
    setInterval: () => 0,
    clearInterval() {},
    setTimeout: () => 0,
    performance: { now: () => 0 },
    navigator: {},
    devicePixelRatio: 1,
    addEventListener() {},
    __TAURI__: {
      core: { invoke: cmd => Promise.resolve(cmd === "get_metrics" ? metrics : sysinfo) },
      window: { getCurrentWindow: () => ({ setAlwaysOnTop() {} }) },
    },
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(script, sandbox);
  for (let i = 0; i < 5; i++) await new Promise(r => setImmediate(r));
  const card = key => created.find(e => e.dataset.key === key);
  return {
    procs: card("procs").kids[".ptb"].innerHTML,
    drives: card("storage").kids[".drives"].innerHTML,
    gpuLine: card("gpu0")?.kids[".gl"].innerHTML,
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
