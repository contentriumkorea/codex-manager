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
import { folderTransferRequest } from "../ui/destination-model.mjs";
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
  getAttribute(key) { return this.attributes[key]; }
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
                request.action === "portable_transfer_plan"
                  ? {
                      revision: "source-r",
                      warning: "whole folder transfer",
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
  for (const id of ["left"]) {
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
    folderTransferRequest,
    destinationBrowser: () => ({path: () => "E:/archive", refresh: async () => {}, navigate: async () => {}}),
    portableUI: () => ({openFolder: async () => {}, controls: async () => {}}),
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
  for (const id of ["left"]) {
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
test("single main frame rejects unregistered windows and initializes globals once", async () => {
  const h = harness();
  assert.throws(() => h.window.codexPaneBridge.register({}));
  assert.throws(() => h.window.codexPaneBridge.register(h.document.getElementById("left-frame").contentWindow));
  await h.api.left.invoke("select_home", {home:"A",pane:"right"});
  await flush();
  assert.equal(h.calls.find(c=>c.name==="select_home").args.pane,"left");
  assert.equal(h.calls.filter(c=>c.name==="ready").length,1);
  assert.equal(h.document.getElementById("split-toggle").getAttribute("aria-pressed"),"false");
  assert.equal(h.document.getElementById("right-pane").hidden,true);
});
test("split toggle persists choice and labels without a second catalog", async () => {
  const h=harness(); await flush();
  await h.document.getElementById("split-toggle").fire("click");
  assert.equal(h.document.getElementById("right-pane").hidden,false);
  assert.equal(h.document.getElementById("split-toggle").getAttribute("aria-label"),"분할 화면 닫기");
  assert.equal(h.calls.find(c=>c.args?.action==="layout_preferences").args.args.enabled,true);
});
test("project copy previews exact folder destination and commits the preview revision",async()=>{
  const h=harness();h.api.left.publish(snapshot("A"));
  const pending=h.api.left.transferOther(true); await flush();
  assert.equal(h.calls.some(c=>c.args?.action==="portable_transfer"),false);
  await h.document.getElementById("dialog-form").fire("submit"); await pending;
  const call=h.calls.find(c=>c.args?.action==="portable_transfer");
  assert.equal(call.args.args.destination,"E:/archive");
  assert.equal(call.args.args.expectedRevision,"source-r");
  assert.equal(call.args.args.copy,true);
});
test("changed main selection prevents committing a stale destination preview",async()=>{
  const h=harness();h.api.left.publish(snapshot("A"));
  const pending=h.api.left.transferOther(false);await flush();
  h.api.left.publish(snapshot("Other"));
  await h.document.getElementById("dialog-form").fire("submit");
  assert.equal(h.document.getElementById("dialog-error").hidden,false);
  h.document.getElementById("dialog-cancel").onclick();await pending;
  assert.equal(h.calls.some(c=>c.args?.action==="portable_transfer"),false);
});
test("file destination transfer uses narrowly scoped external folder action",async()=>{
  const h=harness();h.api.left.publish({...snapshot("A",["A/project/file"]),mode:"files"});
  const pending=h.api.left.transferOther(false);await flush();
  await h.document.getElementById("dialog-form").fire("submit");await pending;
  const call=h.calls.find(c=>c.args?.action==="files_to_folder");
  assert.equal(call.args.args.kind,"move");assert.deepEqual(call.args.args.sources,["A/project/file"]);
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
  const pending = h.api.left.conclude({
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
