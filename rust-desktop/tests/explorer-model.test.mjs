import test from "node:test";
import assert from "node:assert/strict";
import {
  sortedRows,
  selectRange,
  dropPlan,
  formatBytes,
} from "../ui/explorer-model.mjs";
test("sort directions are natural and keep folders before files without modifying input", () => {
  const rows = [
    { id: 1, name: "파일10", directory: false },
    { id: 2, name: "파일2", directory: false },
    { id: 3, name: "폴더", directory: true },
  ];
  assert.deepEqual(
    sortedRows(rows, "name", "desc").map((x) => x.id),
    [3, 1, 2],
  );
  assert.deepEqual(
    sortedRows(rows, "name", "asc").map((x) => x.id),
    [3, 2, 1],
  );
  assert.equal(rows[0].id, 1);
});
test("shift selection spans the visible sorted list and ctrl toggles without clearing", () => {
  const ids = ["c", "b", "a"];
  assert.deepEqual(
    [...selectRange(ids, new Set(["c"]), "a", "c", { shiftKey: true })],
    ids,
  );
  assert.deepEqual(
    [...selectRange(ids, new Set(["c"]), "b", "c", { ctrlKey: true })],
    ["c", "b"],
  );
  assert.deepEqual(
    [...selectRange(ids, new Set(["c", "b"]), "b", "c", { ctrlKey: true })],
    ["c"],
  );
});
test("cross-store and self merges are refused and file copy modifier is retained", () => {
  assert.equal(
    dropPlan(
      { kind: "projects", home: "A", ids: ["a"] },
      { projectId: "b" },
      "B",
    ),
    null,
  );
  assert.equal(
    dropPlan(
      { kind: "projects", home: "A", ids: ["a"] },
      { projectId: "a" },
      "A",
    ),
    null,
  );
  assert.equal(
    dropPlan(
      { kind: "files", home: "A", ids: ["x"] },
      { directory: "d" },
      "A",
      true,
    ).kind,
    "copy",
  );
});
test("byte units include zero, missing data and terabytes", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(undefined), "—");
  assert.equal(formatBytes(1024 ** 4), "1 TB");
});
