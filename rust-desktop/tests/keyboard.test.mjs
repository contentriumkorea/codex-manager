import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { selectRange } from "../ui/explorer-model.mjs";

// Execute the product event handler: a standalone model guard misses DOM type interactions.
const source = readFileSync(new URL("../ui/main.mjs", import.meta.url), "utf8");
const start = source.indexOf("function handleKey(event) {");
const end = source.indexOf(
  'document.addEventListener("keydown", handleKey);',
  start,
);
assert.ok(start >= 0 && end > start);
class Input {
  constructor(type) {
    this.type = type;
  }
}
class Textarea {}
class Select {}
function harness() {
  const actions = [];
  const elements = {
    search: {
      focus: () => actions.push("focus"),
      select: () => actions.push("select"),
    },
    rows: {
      children: [
        { scrollIntoView() {} },
        { scrollIntoView: () => actions.push("scroll") },
      ],
    },
    context: { hidden: true },
  };
  const state = {
    selected: new Set(["a"]),
    anchor: "a",
    busy: false,
    externalBusy: false,
  };
  const context = {
    HTMLInputElement: Input,
    HTMLTextAreaElement: Textarea,
    HTMLSelectElement: Select,
    bridge: {
      isActive: () => true,
      dialogOpen: () => false,
      clearClipboard: () => actions.push("clear"),
    },
    state,
    selectRange,
    el: (id) => elements[id],
    rows: () => [{ id: "a" }, { id: "b" }],
    render: () => actions.push("render"),
    copy: (cut) => actions.push(cut ? "cut" : "copy"),
    paste: () => actions.push("paste"),
    undo: () => actions.push("undo"),
    rename: () => actions.push("rename"),
    deleteItems: () => actions.push("delete"),
    refresh: () => actions.push("refresh"),
    selectedRows: () => [],
    showError: (error) => {
      throw error;
    },
  };
  vm.runInNewContext(source.slice(start, end), context);
  return {
    actions,
    state,
    key(target, key, ctrlKey = false) {
      let prevented = false;
      context.handleKey({
        target,
        key,
        ctrlKey,
        preventDefault: () => {
          prevented = true;
        },
      });
      return prevented;
    },
  };
}
test("focused row checkbox and radio retain clipboard, management and navigation shortcuts", () => {
  for (const type of ["checkbox", "radio"]) {
    const h = harness(),
      target = new Input(type);
    for (const key of ["c", "x", "v", "z", "f"])
      assert.equal(h.key(target, key, true), true);
    for (const key of ["F2", "Delete", "F5", "ArrowDown"])
      assert.equal(h.key(target, key), true);
    assert.deepEqual(h.actions, [
      "copy",
      "cut",
      "paste",
      "undo",
      "focus",
      "select",
      "rename",
      "delete",
      "refresh",
      "render",
      "scroll",
    ]);
    assert.deepEqual([...h.state.selected], ["b"]);
    assert.equal(
      h.key(target, " "),
      false,
      "Space retains the checkbox's native toggle",
    );
  }
});
test("text inputs, textareas, selects and editable content keep their normal editing shortcuts", () => {
  for (const target of [
    new Input("text"),
    new Input("search"),
    new Input("number"),
    new Textarea(),
    new Select(),
    { isContentEditable: true },
  ]) {
    const h = harness();
    for (const key of ["c", "x", "v", "z", "f"])
      assert.equal(h.key(target, key, true), false);
    for (const key of ["F2", "Delete", "F5", "ArrowDown", " "])
      assert.equal(h.key(target, key), false);
    assert.deepEqual(h.actions, []);
    assert.deepEqual([...h.state.selected], ["a"]);
  }
});
