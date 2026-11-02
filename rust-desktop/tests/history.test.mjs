import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
const source = readFileSync(
  new URL("../ui/main.mjs", import.meta.url),
  "utf8",
).replace(/\r\n/g, "\n");
const start = source.indexOf(
  '    if (r.payload.kind === "rust-cross-store") {',
);
const end = source.indexOf(
  '    if (\n      r.payload.kind !== "rust-cross-store"',
  start,
);
assert.ok(start >= 0 && end > start);
function render(state, lastEvent) {
  const buttons = [],
    info = [],
    calls = [];
  const r = {
    state,
    payload: { kind: "rust-cross-store", targetHome: "B" },
    lastEvent,
  };
  vm.runInNewContext(source.slice(start, end), {
    r,
    info: (...v) => info.push(v),
    formatBytes: String,
    body: { append: (...b) => buttons.push(...b) },
    button: (label, click) => ({ label, click }),
    bridge: {
      resume: (r) => calls.push(["resume", r]),
      conclude: (r) => calls.push(["conclude", r]),
    },
  });
  return { buttons, info, calls, r };
}
test("pending cross-store history exposes dedicated safe conclusion; terminal conclusion has no resume", () => {
  const pending = render("needs_recovery");
  assert.deepEqual(
    pending.buttons.map((b) => b.label),
    ["전송 계속", "보존 후 안전 종료…"],
  );
  pending.buttons[1].click();
  assert.deepEqual(pending.calls, [["conclude", pending.r]]);
  assert.deepEqual(
    render("conclusion-needs-recovery").buttons.map((b) => b.label),
    ["보존 후 안전 종료…"],
  );
  const result = render("cancelled", {
    payload: {
      safeConclusion: true,
      retained: {
        retainedStaging: { path: "B/staging" },
        recoveryPath: "B/recovery",
        roots: [
          {
            source: { path: "A/original" },
            target: { path: "B/target" },
            hold: { path: "A/hold" },
            sourceUnavailable: true,
          },
        ],
        conversations: [
          { id: "moved", sourcePresent: false, targetPresent: true },
        ],
      },
    },
  });
  assert.equal(result.buttons.length, 0);
  const text = result.info.flat().join("\n");
  for (const value of [
    "전송 완료·원상복구가 아닙니다",
    "B/staging",
    "B/recovery",
    "A/hold",
    "원래 위치의 파일을 사용할 수 없음",
    "원본에 없는 대화",
  ])
    assert.ok(text.includes(value), value);
});
