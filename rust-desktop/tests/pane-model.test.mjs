import test from "node:test";
import assert from "node:assert/strict";
import { PaneModel, sameHome, keyboardTarget } from "../ui/pane-model.mjs";
const left = {
  home: "C:\\Store A",
  mode: "projects",
  project: null,
  directory: "",
  sort: { key: "name", direction: "asc" },
  ids: ["p1"],
  revision: "a",
};
const right = {
  home: "D:\\Store B",
  mode: "threads",
  project: "p2",
  directory: "D:\\work",
  sort: { key: "size", direction: "desc" },
  ids: ["t1"],
  revision: "b",
};
test("pane home, sort, navigation and selection remain independent", () => {
  const model = new PaneModel();
  model.update("left", left);
  model.update("right", right);
  left.ids.push("p3");
  left.sort.direction = "desc";
  assert.deepEqual(model.snapshot("left").ids, ["p1"]);
  assert.equal(model.snapshot("left").sort.direction, "asc");
  model.update("right", { ...right, home: "E:\\Other", ids: [] });
  assert.equal(model.snapshot("left").home, "C:\\Store A");
  assert.equal(model.snapshot("right").home, "E:\\Other");
});
test("keyboard routes only to active pane and preserves editing and dialog shortcuts", () => {
  assert.equal(
    keyboardTarget("right", { editing: false, dialog: false }),
    "right",
  );
  assert.equal(keyboardTarget("left", { editing: true, dialog: false }), null);
  assert.equal(keyboardTarget("right", { editing: false, dialog: true }), null);
});
test("trusted drag and clipboard pin source identity across active-pane changes", () => {
  const model = new PaneModel();
  model.update("left", { ...left, ids: ["p1"] });
  model.update("right", right);
  const drag = model.beginDrag("left");
  model.active = "right";
  assert.equal(model.dragSource(drag).pane, "left");
  assert.equal(model.dragSource("forged"), null);
  model.copy("left", false);
  const copy = model.clipboard;
  assert.equal(copy.home, left.home);
  assert.equal(copy.copy, true);
  model.copy("left", true);
  assert.equal(model.clipboard.copy, false);
  assert.equal(model.clipboard.pane, "left");
  model.update("left", { ...left, home: "Z:\\new", ids: ["different"] });
  assert.deepEqual(copy.ids, ["p1"]);
  assert.equal(model.validSource(copy), false);
});
test("preview invalidates when source selection or destination navigation changes, not another active pane", () => {
  const model = new PaneModel();
  model.update("left", { ...left, ids: ["p1"] });
  model.update("right", right);
  const source = model.capture("left");
  const ticket = model.ticket(source, "right");
  model.active = "right";
  assert.equal(model.valid(ticket), true);
  model.update("right", { ...right, directory: "D:\\changed" });
  assert.equal(model.valid(ticket), false);
  const next = model.ticket(source, "right");
  model.update("left", { ...left, ids: ["p9"] });
  assert.equal(model.valid(next), false);
});
test("async tickets reject old store generations and normalize Windows store aliases", () => {
  const model = new PaneModel();
  model.update("left", { ...left, ids: ["p1"] });
  const ticket = model.generation("left");
  model.update("left", { ...left, home: "E:\\store" });
  assert.equal(model.current("left", ticket), false);
  assert.equal(sameHome("C:/Store A/", "c:\\store a"), true);
  assert.equal(sameHome("C:\\A", "C:\\AB"), false);
});
