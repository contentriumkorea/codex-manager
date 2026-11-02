import test from "node:test";
import assert from "node:assert/strict";
import { portableUI } from "../ui/portable-ui.mjs";
import { visibleFolders, folderTransferRequest } from "../ui/destination-model.mjs";

class Element {
  constructor(tag, text = "") { this.tagName = tag; this.textContent = text; this.children = []; this.value = ""; this.events = {}; this.isConnected = true; this.open = true; }
  append(...items) { this.children.push(...items); }
  setAttribute() {}
  addEventListener(event, handler) { this.events[event] = handler; }
  fire(event) { return this.events[event]?.({ target: this }); }
}
function harness({ changed = false, corrupt = false, status = null } = {}) {
  const calls = [], elements = new Map(), errors = [];
  let collect, resolveDialog, body, planCount = 0;
  const element = (id) => { if (!elements.has(id)) elements.set(id, new Element(id)); return elements.get(id); };
  const plan = { namespace: "n", snapshotId: "one", anchor: "E:/folder", roots: ["E:/folder"], project: { name: "Saved" }, conversationCount: 2, conflicts: [], skipped: [], canRestore: true };
  const job = async (action, args) => {
    calls.push({ action, args });
    if (action === "portable_discover") return { projects: [{ anchor: plan.anchor, namespace: "n", complete: true, project: { name: "Saved" } }], diagnostics: [] };
    if (action === "portable_inspect") { if (corrupt) throw new Error("corrupt snapshot"); return { project: { name: "Saved" } }; }
    if (action === "portable_plan") { planCount++; return { ...plan, snapshotId: changed && planCount > 1 ? "two" : "one" }; }
    if (action === "portable_restore") return { projectId: "restored" };
    if (action === "portable_status") return (typeof status === "function" ? status(args.anchor) : status) ?? { registration: { enabled: true, error: "last attempt failed" }, snapshot: { complete: true, lastSuccessfulSave: "2026-11-01" } };
    return {};
  };
  const ui = portableUI({
    dialog: (owner, title, build) => { body = new Element("body"); collect = build(body); return new Promise((resolve) => { resolveDialog = resolve; }); },
    job, run: (owner, action, args) => job(action, args), invoke: async () => "E:/folder",
    node: (tag, text) => new Element(tag, text), snapshot: () => ({ home: "store", projects: [{ id: "existing", name: "Existing" }] }),
    error: (e) => errors.push(String(e)), element,
  });
  return { ui, calls, element, errors, body: () => body, confirm: () => resolveDialog(collect()), cancel: () => resolveDialog(null) };
}
const flush = () => new Promise((resolve) => setImmediate(resolve));
test("folder search handles long Unicode names and cannot transfer a thread to a file folder", () => {
  const long = "가나다".repeat(100);
  assert.deepEqual(visibleFolders([{ name: long, path: "D:/" }, { name: "Other", path: "E:/" }], "가나다"), [{ name: long, path: "D:/" }]);
  assert.throws(() => folderTransferRequest({ kind: "threads", ids: ["t"] }, "D:/", false), /메인/);
  assert.equal(folderTransferRequest({ kind: "files", ids: ["f"], home: "h" }, "D:/", true).action, "files_to_folder");
});
test("folder opening verifies inspection and plans twice then restores selected folder in place", async () => {
  const h = harness(); const pending = h.ui.openFolder(); await flush();
  const select = h.body().children[0]; select.value = "0"; await select.fire("change");
  assert.equal(h.element("dialog-ok").disabled, false);
  assert.equal(h.calls.some((c) => c.action === "portable_restore"), false);
  h.confirm(); assert.equal((await pending).projectId, "restored");
  assert.equal(h.calls.filter((c) => c.action === "portable_plan").length, 2);
  assert.equal(h.calls.at(-1).args.anchor, "E:/folder");
  assert.equal(h.calls.at(-1).args.target, null);
});
test("changed snapshot after confirmation is not restored", async () => {
  const h = harness({ changed: true }); const pending = h.ui.openFolder(); const rejection = assert.rejects(pending, /바뀌/); await flush();
  h.body().children[0].value = "0"; await h.body().children[0].fire("change"); h.confirm(); await rejection;
  assert.equal(h.calls.some((c) => c.action === "portable_restore"), false);
});
test("corrupt portable contents cannot enable restore", async () => {
  const h = harness({ corrupt: true }); const pending = h.ui.openFolder(); await flush();
  h.body().children[0].value = "0"; await h.body().children[0].fire("change");
  assert.equal(h.element("dialog-ok").disabled, true);
  assert.match(h.body().children[2].textContent, /corrupt/);
  h.cancel(); await pending; assert.equal(h.calls.some((c) => c.action === "portable_restore"), false);
});
test("automatic controls show a later failure even while the last good snapshot remains complete", async () => {
  const h = harness(); const project = { id: "p", name: "P", roots: [{ path: "E:/folder" }] };
  assert.match(await h.ui.summary(project), /last attempt failed/);
  const pending = h.ui.controls(project); await flush();
  const text = h.body().children.map((n) => n.textContent).join("\n");
  assert.ok(text.includes(new Date("2026-11-01").toLocaleString())); assert.match(text, /last attempt failed/);
  h.confirm(); await pending;
  assert.equal(h.calls.at(-1).action, "portable_configure"); assert.equal(h.calls.at(-1).args.enabled, true);
});
const unreadableStatus = {
  registration: { enabled: true, lastSuccessfulSave: "2026-11-01T00:00:00Z", error: null },
  snapshot: null,
  statusError: "Snapshot manifest hash mismatch",
};
test("summary exposes current native status error while preserving historical success and enabled state", async () => {
  const h = harness({ status: unreadableStatus });
  const text = await h.ui.summary({ id: "p", roots: [{ path: "E:/folder" }] });
  assert.match(text, /현재 보관 상태 확인 실패: Snapshot manifest hash mismatch/);
  assert.ok(text.includes(new Date("2026-11-01T00:00:00Z").toLocaleString()));
  assert.match(text, /자동 보관 켜짐/);
});
test("controls expose current native status error without erasing historical success or opt-in", async () => {
  const h = harness({ status: unreadableStatus });
  const pending = h.ui.controls({ id: "p", name: "P", roots: [{ path: "E:/folder" }] });
  await flush();
  try {
    const text = h.body().children.map((n) => n.textContent).join("\n");
    assert.match(text, /현재 보관 상태 확인 실패: Snapshot manifest hash mismatch/);
    assert.ok(text.includes(new Date("2026-11-01T00:00:00Z").toLocaleString()));
    assert.equal(h.body().children[2].children[0].checked, true);
  } finally { h.cancel(); await pending; }
});

test("summary follows the configured current anchor and shows its retention or actual error", async () => {
  for (const failed of [false, true]) {
    const h = harness({ status: (anchor) => ({ registration: { anchor: "D:/second", enabled: true }, snapshot: anchor === "D:/second" ? { lastSuccessfulSave: "2026-11-01T03:00:00Z", lastAttempt: { retentionWarning: "edited payload retained" } } : null, statusError: anchor === "D:/first" ? "wrong root" : failed ? "actual error" : null }) });
    const text = await h.ui.summary({ id: "p", roots: [{ path: "D:/first" }, { path: "D:/second" }] });
    assert.deepEqual(h.calls.map(c => c.args.anchor), ["D:/first", "D:/second"]);
    assert.ok(text.includes(new Date("2026-11-01T03:00:00Z").toLocaleString()));
    assert.match(text, /edited payload retained/);
    assert.equal(text.includes("actual error"), failed);
    assert.equal(text.includes("wrong root"), false);
  }
});
test("removed anchor falls back to current root and a never-saved source is clean", async () => {
  const h = harness({ status: { registration: { anchor: "D:/removed" }, snapshot: { state: "notSaved", complete: false }, statusError: null } });
  const text = await h.ui.summary({ id: "p", roots: [{ path: "D:/current" }] });
  assert.deepEqual(h.calls.map(c => c.args.anchor), ["D:/current"]);
  assert.match(text, /아직 없음/); assert.equal(text.includes("실패"), false);
});
test("display preserves invalid timestamps and converts epoch seconds", async () => {
  for (const value of [1735689600, "old-success-value"]) {
    const h = harness({ status: { registration: { lastSuccessfulSave: value } } });
    const text = await h.ui.summary({ id: "p", roots: [{ path: "D:/current" }] });
    assert.ok(text.includes(typeof value === "number" ? new Date(value * 1000).toLocaleString() : value));
  }
});
