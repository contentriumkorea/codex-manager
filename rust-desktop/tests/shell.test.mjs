import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import {
  PaneModel,
  sameHome,
  normalizeHome,
  keyboardTarget,
} from "../ui/pane-model.mjs";
import { dropPlan, formatBytes, duration } from "../ui/explorer-model.mjs";
class Element {
  constructor(tag = "div") {
    this.tagName = tag;
    this.listeners = new Map();
    this.children = [];
    this.attributes = {};
    this.classList = { toggle() {} };
    this.style = { setProperty() {} };
    this.open = false;
    this.textContent = "";
    this.value = "";
    this.disabled = false;
  }
  append(...children) {
    this.children.push(...children);
  }
  replaceChildren(...children) {
    this.children = children;
  }
  setAttribute(key, value) {
    this.attributes[key] = String(value);
  }
  removeAttribute(key) {
    delete this.attributes[key];
  }
  addEventListener(type, fn) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
  }
  async fire(type, event = {}) {
    for (const fn of this.listeners.get(type) ?? [])
      await fn({ target: this, preventDefault() {}, ...event });
    if (type === "submit") await this.onsubmit?.({ preventDefault() {} });
  }
  showModal() {
    this.open = true;
  }
  close() {
    this.open = false;
    this.onclose?.();
  }
  matches() {
    return false;
  }
}
function harness() {
  const elements = new Map();
  const document = {
    getElementById: (id) => {
      if (!elements.has(id)) elements.set(id, new Element());
      return elements.get(id);
    },
    createElement: (tag) => new Element(tag),
    addEventListener() {},
  };
  const window = {
    __TAURI__: {
      core: {
        invoke: async (name, args) => {
          calls.push({ name, args });
          if (name === "start_job") {
            const id = "job-" + jobs.size;
            jobs.set(id, { action: args.action, args: args.args });
            return id;
          }
          if (name === "job_status") {
            const request = jobs.get(args.id);
            return {
              state: "completed",
              elapsed: 1,
              phase: "done",
              result:
                request.action === "cross_store_plan"
                  ? {
                      request: request.args,
                      sourceRevision: "source-r",
                      targetRevision: "target-r",
                      projects: [],
                      threads: [],
                      roots: [],
                      warnings: [],
                    }
                  : request.action === "cross_store_conclude_plan"
                    ? {
                        revision: "conclusion-r",
                        warning: "안전 종료는 전송 완료나 원상복구가 아닙니다.",
                        roots: [
                          {
                            source: { path: "A/source", exists: false },
                            target: { path: "B/target", exists: true },
                            hold: { path: "A/hold", exists: true },
                            action: "retain-current",
                            sourceUnavailable: true,
                          },
                        ],
                        conversations: [
                          {
                            id: "moved",
                            sourcePresent: false,
                            targetPresent: true,
                          },
                        ],
                        retainedStaging: { path: "B/staging" },
                        recoveryPath: "B/recovery",
                        childJournals: [{ path: "manager/source.sqlite" }],
                      }
                    : {},
            };
          }
        },
      },
      event: { listen() {} },
    },
  };
  const calls = [],
    jobs = new Map();
  for (const id of ["left", "right"]) {
    const frame = document.getElementById(id + "-frame");
    frame.contentWindow = {};
    Object.defineProperty(frame, "src", {
      set(value) {
        assert.ok(
          window.codexPaneBridge,
          "bridge must exist before iframe src",
        );
        frame.source = value;
      },
    });
  }
  const context = {
    document,
    window,
    PaneModel,
    sameHome,
    normalizeHome,
    keyboardTarget,
    dropPlan,
    formatBytes,
    duration,
    setTimeout,
    queueMicrotask,
    console,
  };
  const source = readFileSync(
    new URL("../ui/shell.mjs", import.meta.url),
    "utf8",
  ).replace(/^import[\s\S]*?from\s+['"][^'"]+['"];?\r?\n/gm, "");
  vm.runInNewContext(source, context);
  const controllers = {},
    api = {};
  for (const id of ["left", "right"]) {
    api[id] = window.codexPaneBridge.register(
      document.getElementById(id + "-frame").contentWindow,
    );
    controllers[id] = {
      refresh: async () => {},
      render() {},
      setBusy() {},
      setDestination() {},
      dragEnded() {},
      reloadStores: async () => {},
      performDrop: async () => {},
      startGlobals: async () => {},
      action: async () => {},
      key() {},
    };
    api[id].ready(controllers[id], { version: "test" });
  }
  return { window, document, elements, calls, jobs, api, controllers };
}
const snapshot = (home, ids = ["p"]) => ({
  home,
  mode: "projects",
  project: null,
  directory: "",
  sort: { key: "name", direction: "asc" },
  ids,
  revision: "r",
  projects: [],
  destination: "E:\\workspace",
  savedDestination: "E:\\workspace",
});
const flush = async () => {
  for (let i = 0; i < 8; i++)
    await new Promise((resolve) => setImmediate(resolve));
};
test("bridge registers exact frame windows, injects independent pane identity and initializes globals once", async () => {
  const h = harness();
  assert.throws(() => h.window.codexPaneBridge.register({}));
  assert.throws(() =>
    h.window.codexPaneBridge.register(
      h.document.getElementById("left-frame").contentWindow,
    ),
  );
  await h.api.right.invoke("bootstrap", { pane: "left" });
  await h.api.left.invoke("select_home", { home: "A", pane: "right" });
  await flush();
  assert.equal(h.calls.find((c) => c.name === "bootstrap").args.pane, "right");
  assert.equal(h.calls.find((c) => c.name === "select_home").args.pane, "left");
  assert.equal(h.calls.filter((c) => c.name === "ready").length, 1);
});
test("cross-pane copy previews pinned source home and commits only returned revisions", async () => {
  const h = harness();
  h.api.left.publish(snapshot("A"));
  h.api.right.publish(snapshot("B", []));
  h.api.left.activate();
  h.api.left.copy(false);
  h.api.right.activate();
  const pending = h.api.right.paste();
  await flush();
  const preview = h.calls.find((c) => c.args?.action === "cross_store_plan");
  assert.equal(preview.args.args.home, "A");
  assert.equal(preview.args.args.targetHome, "B");
  assert.equal(preview.args.args.copy, true);
  assert.equal(
    h.calls.some((c) => c.args?.action === "cross_store_transfer"),
    false,
  );
  await h.document.getElementById("dialog-form").fire("submit");
  await pending;
  const commit = h.calls.find((c) => c.args?.action === "cross_store_transfer");
  assert.equal(commit.args.args.expectedRevision, "source-r");
  assert.equal(commit.args.args.targetRevision, "target-r");
  assert.equal(commit.args.args.home, "A");
});
test("source/target changes invalidate visible preview and cannot execute an obsolete commit", async () => {
  const h = harness();
  h.api.left.publish(snapshot("A"));
  h.api.right.publish(snapshot("B", []));
  const pending = h.api.left.transferOther(false);
  await flush();
  h.api.right.publish({ ...snapshot("B", []), directory: "changed" });
  await h.document.getElementById("dialog-form").fire("submit");
  assert.equal(
    h.calls.some((c) => c.args?.action === "cross_store_transfer"),
    false,
  );
  assert.match(
    h.document.getElementById("dialog-error").textContent,
    /変|바뀌|미리보기/,
  );
  h.document.getElementById("dialog-cancel").onclick();
  await pending;
});
test("forged drag is refused and interrupted transfer refreshes both affected homes", async () => {
  const h = harness();
  h.api.left.publish(snapshot("A"));
  h.api.right.publish(snapshot("B", []));
  await assert.rejects(h.api.right.drop("forged", {}, false), /선택/);
  const refreshed = [];
  h.controllers.left.refresh = async () => refreshed.push("left");
  h.controllers.right.refresh = async () => refreshed.push("right");
  h.window.__TAURI__.core.invoke = async (name, args) => {
    if (name === "start_job") return "failed-job";
    if (name === "job_status")
      return { state: "cancelled", error: "cancelled" };
  };
  await assert.rejects(
    h.api.left.runVisible("cross_store_transfer", {
      home: "A",
      targetHome: "B",
    }),
    /cancelled/,
  );
  assert.deepEqual(refreshed.sort(), ["left", "right"]);
});
test("late preview cannot enable confirmation in a newer global dialog", async () => {
  const h = harness();
  h.api.left.publish(snapshot("A"));
  h.api.right.publish(snapshot("B", []));
  const original = h.window.__TAURI__.core.invoke;
  let resolvePlan;
  h.window.__TAURI__.core.invoke = async (name, args) => {
    if (name === "start_job" && args.action === "cross_store_plan")
      return "slow-plan";
    if (name === "job_status" && args.id === "slow-plan")
      return new Promise((resolve) => {
        resolvePlan = resolve;
      });
    return original(name, args);
  };
  const pending = h.api.left.transferOther(false);
  await flush();
  h.document.getElementById("dialog-cancel").onclick();
  await pending;
  const newer = h.api.right.dialog("new dialog", () => {
    h.document.getElementById("dialog-ok").disabled = true;
    return () => true;
  });
  resolvePlan({
    state: "completed",
    result: {
      request: {},
      sourceRevision: "a",
      targetRevision: "b",
      roots: [],
    },
  });
  await flush();
  assert.equal(h.document.getElementById("dialog-ok").disabled, true);
  h.document.getElementById("dialog-cancel").onclick();
  await newer;
});
test("matching-home background size jobs are shared without taking visible progress", async () => {
  const h = harness();
  let complete;
  const original = h.window.__TAURI__.core.invoke;
  h.window.__TAURI__.core.invoke = async (name, args) => {
    if (name === "start_job" && args.action === "sizes") {
      h.calls.push({ name, args });
      return "sizes";
    }
    if (name === "job_status" && args.id === "sizes")
      return new Promise((resolve) => {
        complete = resolve;
      });
    return original(name, args);
  };
  const first = h.api.left.sizes("C:/Store/");
  const second = h.api.right.sizes("c:\\store");
  await flush();
  assert.equal(h.calls.filter((c) => c.args?.action === "sizes").length, 1);
  complete({ state: "completed", result: { p: { bytes: 10 } } });
  assert.deepEqual(await first, await second);
  assert.equal(h.document.getElementById("progress-phase").textContent, "");
});
test("store changes while saving destination cannot commit the earlier preview", async () => {
  const h = harness();
  h.api.left.publish(snapshot("A"));
  h.api.right.publish(snapshot("B", []));
  const original = h.window.__TAURI__.core.invoke;
  let save;
  h.window.__TAURI__.core.invoke = async (name, args) => {
    if (name === "start_job" && args.action === "pane_preferences")
      return "slow-save";
    if (name === "job_status" && args.id === "slow-save")
      return new Promise((resolve) => {
        save = resolve;
      });
    return original(name, args);
  };
  const pending = h.api.left.transferOther(false);
  const rejection = assert.rejects(pending, /바뀌/);
  await flush();
  await h.document.getElementById("dialog-form").fire("submit");
  await flush();
  h.api.left.publish(snapshot("Other"));
  save({ state: "completed", result: {} });
  await rejection;
  assert.equal(
    h.calls.some((c) => c.args?.action === "cross_store_transfer"),
    false,
  );
});
test("native ready is reported once while first-run legal consent remains unresolved", async () => {
  const h = harness();
  let resolveConsent,
    consentStarted = false,
    consentFinished = false;
  h.controllers.left.startGlobals = async () => {
    consentStarted = true;
    await new Promise((resolve) => {
      resolveConsent = resolve;
    });
    consentFinished = true;
  };
  await flush();
  assert.equal(consentStarted, true);
  assert.equal(consentFinished, false);
  assert.equal(h.calls.filter((call) => call.name === "ready").length, 1);
  h.api.left.ready(h.controllers.left, { version: "test" });
  await flush();
  assert.equal(h.calls.filter((call) => call.name === "ready").length, 1);
  assert.equal(consentFinished, false);
  resolveConsent();
  await flush();
  assert.equal(consentFinished, true);
});

test("safe conclusion previews retained paths and unavailable originals, then commits exact preview revision", async () => {
  const h = harness();
  h.api.left.publish(snapshot("A"));
  h.api.right.publish(snapshot("B"));
  const pending = h.api.left.conclude({
    id: "operation",
    payload: { sourceHome: "A", targetHome: "B" },
  });
  await flush();
  assert.equal(
    h.calls.some((c) => c.args?.action === "cross_store_conclude"),
    false,
  );
  const preview = h.calls.find(
    (c) => c.args?.action === "cross_store_conclude_plan",
  );
  assert.equal(preview.args.args.home, "A");
  assert.equal(preview.args.args.targetHome, "B");
  const text = h.document
    .getElementById("dialog-content")
    .children.map((n) => n.textContent)
    .join("\n");
  for (const value of [
    "원상복구가 아닙니다",
    "A/source",
    "B/target",
    "A/hold",
    "B/staging",
    "B/recovery",
    "원래 위치의 파일을 사용할 수 없음",
    "원본 없음 / 대상 있음",
  ])
    assert.ok(text.includes(value), value);
  await h.document.getElementById("dialog-form").fire("submit");
  await pending;
  const commit = h.calls.find((c) => c.args?.action === "cross_store_conclude");
  assert.equal(commit.args.args.expectedRevision, "conclusion-r");
  assert.equal(commit.args.args.id, "operation");
});
test("cancelling safe conclusion preview never mutates either store", async () => {
  const h = harness();
  const pending = h.api.right.conclude({
    id: "operation",
    payload: { sourceHome: "A", targetHome: "B" },
  });
  await flush();
  h.document.getElementById("dialog-cancel").onclick();
  await pending;
  assert.equal(
    h.calls.some((c) => c.args?.action === "cross_store_conclude"),
    false,
  );
});
