import {
  PaneModel,
  sameHome,
  normalizeHome,
  keyboardTarget,
} from "./pane-model.mjs";
import { dropPlan, formatBytes, duration } from "./explorer-model.mjs";
import { folderTransferRequest } from "./destination-model.mjs";
import { destinationBrowser } from "./destination-ui.mjs";
import { portableUI } from "./portable-ui.mjs";
const el = (id) => document.getElementById(id);
const invoke = (name, args = {}) => window.__TAURI__.core.invoke(name, args);
const model = new PaneModel();
const panes = new Map();
const sizeJobs = new Map();
const frames = { left: el("left-frame") };
let visibleJob = null,
  dialogFinish = null,
  initializing = false,
  dialogSequence = 0;
const node = (tag, text, cls) => {
  const n = document.createElement(tag);
  if (text !== undefined) n.textContent = text;
  if (cls) n.className = cls;
  return n;
};
function field(label, value = "") {
  const box = node("label", undefined, "field");
  const input = node("input");
  input.value = value;
  input.setAttribute("aria-label", label);
  box.append(node("span", label), input);
  return { box, input };
}
function setStatus(text) {
  el("shell-status").textContent = text;
  el("shell-status").title = text;
}
function activate(pane) {
  model.active = "left";
  for (const id of ["left", "right"])
    el(id + "-pane").classList.toggle("active", id === pane);
  el("active-label").textContent =
    "메인 화면의 프로젝트·파일을 선택하세요";
}
function dialog(owner, title, build, ok = "확인", danger = false) {
  if (dialogFinish)
    return Promise.reject(new Error("열린 확인 창을 먼저 마무리하세요."));
  ++dialogSequence;
  el("dialog-title").textContent = title;
  el("dialog-content").replaceChildren();
  el("dialog-error").hidden = true;
  el("dialog-ok").textContent = ok;
  el("dialog-ok").className = danger ? "danger" : "primary";
  el("dialog-ok").hidden = false;
  el("dialog-cancel").textContent = "취소";
  el("dialog-ok").disabled = false;
  const collect = build(el("dialog-content"));
  return new Promise((resolve) => {
    const finish = (value) => {
      if (dialogFinish !== finish) return;
      dialogFinish = null;
      el("dialog").onclose = null;
      el("dialog").close();
      resolve(value);
    };
    dialogFinish = finish;
    el("dialog").onclose = () => {
      if (!el("dialog").open) finish(null);
    };
    el("dialog-cancel").onclick = () => finish(null);
    el("dialog-close").onclick = () => finish(null);
    el("dialog-form").onsubmit = async (event) => {
      event.preventDefault();
      el("dialog-error").hidden = true;
      el("dialog-ok").disabled = true;
      try {
        finish((await collect()) ?? true);
      } catch (error) {
        if (dialogFinish !== finish) return;
        el("dialog-error").textContent = String(error);
        el("dialog-error").hidden = false;
      } finally {
        if (dialogFinish === finish) el("dialog-ok").disabled = false;
      }
    };
    el("dialog").showModal();
  });
}
async function showError(error) {
  setStatus(String(error));
  if (dialogFinish) {
    el("dialog-error").textContent = String(error);
    el("dialog-error").hidden = false;
    return;
  }
  await dialog(
    "shell",
    "작업을 완료하지 못했습니다.",
    (body) => {
      body.append(node("div", String(error), "summary-box"));
      return () => true;
    },
    "닫기",
  );
}
function progress(p) {
  el("progress").hidden = false;
  el("progress-phase").textContent = p.phase || "작업 준비";
  el("progress-path").textContent = p.path ?? "";
  const percent =
    p.total > 0 ? Math.min(99, Math.floor((p.done / p.total) * 100)) : null;
  el("progress-percent").textContent =
    p.state === "completed"
      ? "완료"
      : p.state === "failed"
        ? "실패"
        : p.state === "cancelled"
          ? "취소됨"
          : percent === null
            ? "작업 중"
            : percent + "%";
  if (p.state === "completed") el("progress-bar").value = 100;
  else if (percent === null) el("progress-bar").removeAttribute("value");
  else el("progress-bar").value = percent;
  el("progress-stats").textContent =
    `경과 ${duration(p.elapsed ?? 0)}${p.total > 0 ? " · " + (p.unit === "items" ? p.done + " / " + p.total + "개" : formatBytes(p.done) + " / " + formatBytes(p.total)) : ""}${p.speed > 0 ? " · " + (p.unit === "items" ? p.speed.toFixed(1) + "개" : formatBytes(p.speed)) + "/초" : ""}${p.remaining > 0 && p.speed > 0 ? " · 약 " + duration(p.remaining) + " 남음" : ""}`;
  el("cancel").disabled = p.state !== "running";
}
async function poll(id, show = false, onProgress) {
  for (;;) {
    const p = await invoke("job_status", { id });
    if (show && visibleJob?.id === id) progress(p);
    onProgress?.(p);
    if (p.state === "completed") return p.result;
    if (["failed", "cancelled"].includes(p.state))
      throw new Error(
        p.error ||
          "작업이 중단됐습니다. 작업 내역에서 결과와 복구를 확인하세요.",
      );
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}
async function refreshHomes(homes) {
  await Promise.all(
    [...panes].map(async ([id, entry]) => {
      if (homes.some((home) => sameHome(home, model.snapshot(id)?.home)))
        await entry.controller.refresh(false);
    }),
  );
}
function setBusy(value) {
  for (const entry of panes.values()) entry.controller?.setBusy(value);
  el("transfer-move").disabled = value;
  el("transfer-copy").disabled = value;
  el("destination-copy").disabled = value;
  el("destination-move").disabled = value;
}
async function runVisible(owner, action, args, onProgress) {
  if (visibleJob) throw new Error("진행 중인 작업이 있습니다.");
  visibleJob = { owner, id: null };
  setBusy(true);
  el("cancel").textContent = "취소";
  progress({ state: "running", phase: "작업 준비", elapsed: 0 });
  const homes = [args.home, args.targetHome].filter(Boolean);
  try {
    const id = await invoke("start_job", { action, args });
    visibleJob.id = id;
    const result = await poll(id, true, onProgress);
    setStatus("작업을 완료했습니다.");
    if (result?.retainedRecovery) await showRecovery(result.retainedRecovery);
    return result;
  } catch (error) {
    setStatus(String(error));
    el("progress-phase").textContent = "작업 결과 확인 · " + String(error);
    el("cancel").disabled = true;
    throw error;
  } finally {
    visibleJob = null;
    setBusy(false);
    await refreshHomes(homes);
  }
}
async function showRecovery(recovery) {
  await dialog(
    "shell",
    "전송 완료 · 복구 사본 보관",
    (body) => {
      body.append(
        node("p", "원본 대화 복구용 사본을 대상 볼륨에 보관했습니다."),
        node(
          "div",
          formatBytes(recovery.bytes) + "\n" + recovery.path,
          "summary-box",
        ),
      );
      return () => true;
    },
    "닫기",
  );
}
async function job(action, args) {
  return poll(await invoke("start_job", { action, args }));
}
function sourceForClipboard() {
  const clip = model.clipboard;
  if (!clip) return null;
  const now = model.snapshot(clip.pane);
  if (!sameHome(now?.home, clip.home))
    throw new Error("원본 저장소가 바뀌었습니다. 다시 복사하거나 잘라내세요.");
  return { ...clip, generation: model.generation(clip.pane) };
}
function targetFor(pane, target = {}) {
  const snapshot = model.snapshot(pane);
  return {
    projectId:
      target.projectId === undefined
        ? snapshot?.targetProject
        : target.projectId,
    directory:
      target.directory ||
      snapshot?.destination ||
      snapshot?.savedDestination ||
      "",
  };
}
async function transfer(source, targetPane, target = {}, copy = false) {
  if (!source || !model.validSource(source)) throw new Error("원본 선택이 바뀌었습니다. 다시 선택하세요.");
  if (visibleJob) throw new Error("진행 중인 작업이 있습니다.");
  const destinationPane = model.snapshot("left");
  if (!sameHome(source.home, destinationPane?.home)) throw new Error("현재 저장소의 항목을 선택하세요.");
  const destination = targetFor("left", target);
  const action = dropPlan(source, destination, destinationPane.home, copy);
  if (!action) throw new Error("대상 프로젝트 또는 파일 폴더를 선택하세요.");
  await panes.get("left").controller.performDrop(action);
  model.clipboard = null;
}
async function transferOther(copy = false) {
  const source = model.capture("left");
  if (!source) throw new Error("옮길 항목을 먼저 선택하세요.");
  return transferToFolder(source, copy);
}
async function transferToFolder(source, copy) {
  const request = folderTransferRequest(source, destination.path(), copy);
  if (!model.validSource(source)) throw new Error("원본 선택이 바뀌었습니다. 다시 선택하세요.");
  let plan = null;
  if (source.kind === "projects") plan = await job("portable_transfer_plan", request.args);
  const accepted = await dialog("left", copy ? "대상 폴더로 복사" : "대상 폴더로 이동", (body) => {
    body.append(node("pre", `${(source.names ?? source.ids).join("\n")}\n→ ${request.args.destination}`, "summary-box transfer-mapping"));
    if (plan) {
      body.append(node("p", plan.warning));
      for (const p of plan.projects) body.append(node("pre", `${p.name} · 대화 ${p.conversationCount}개\n${p.anchor}\n→ ${p.destination}${copy ? "" : "\n" + p.destinations.join("\n")}`, "transfer-mapping"));
    } else body.append(node("p", "기존 파일은 덮어쓰지 않습니다. 이동·복사 결과는 작업 내역에서 되돌릴 수 있습니다."));
    return () => { if (!model.validSource(source)) throw new Error("원본 선택이 바뀌었습니다."); return true; };
  }, copy ? "복사" : "이동");
  if (!accepted) return;
  if (!model.validSource(source)) throw new Error("원본 선택이 바뀌었습니다.");
  if (plan) request.args.expectedRevision = plan.revision;
  await runVisible("left", request.action, request.args);
  model.clipboard = null;
  await destination.refresh();
}
const destination = destinationBrowser({ element: el, invoke, job, error: showError, transfer: transferOther });
const portable = portableUI({ dialog, job, run: runVisible, invoke, node, snapshot: () => model.snapshot("left"), error: showError, element: el });
function showSplit(enabled) {
  el("explorers").classList.toggle("split", enabled);
  el("right-pane").hidden = !enabled; el("splitter").hidden = !enabled;
  el("split-toggle").setAttribute("aria-pressed", String(enabled));
  el("split-toggle").setAttribute("aria-label", enabled ? "분할 화면 닫기" : "분할 화면 열기");
  el("split-toggle").title = enabled ? "분할 화면 닫기" : "분할 화면 열기";
}
el("split-toggle").addEventListener("click", async () => {
  const enabled = el("split-toggle").getAttribute("aria-pressed") !== "true";
  try { await job("layout_preferences", { enabled }); showSplit(enabled); } catch (e) { showError(e); }
});
el("global-open-folder").addEventListener("click", async () => {
  try { const result = await portable.openFolder(); if (result?.projectId) await panes.get("left")?.controller?.openProject(result.projectId, "files"); } catch (e) { showError(e); }
});
const sharedIds = new Set([
  "progress",
  "progress-phase",
  "progress-percent",
  "progress-bar",
  "progress-path",
  "progress-stats",
  "cancel",
  "updates",
  "version",
  "update-banner",
  "update-text",
  "update-now",
]);
// Frames receive only closures bound to their registered Window, never URL/message-supplied pane IDs.
window.codexPaneBridge = Object.freeze({
  register(child) {
    const pane = Object.keys(frames).find(
      (id) => frames[id].contentWindow === child,
    );
    if (!pane || panes.has(pane))
      throw new Error("등록되지 않은 탐색기입니다.");
    const entry = { controller: null, boot: null };
    panes.set(pane, entry);
    return Object.freeze({
      pane,
      element: (id) => (sharedIds.has(id) ? el(id) : null),
      sizes: (home) => {
        const key = normalizeHome(home);
        if (!sizeJobs.has(key))
          sizeJobs.set(
            key,
            job("sizes", { home }).finally(() => sizeJobs.delete(key)),
          );
        return sizeJobs.get(key);
      },
      invoke: (name, args = {}) =>
        invoke(
          name,
          ["bootstrap", "select_home"].includes(name)
            ? { ...args, pane }
            : args,
        ),
      publish: (snapshot) => model.update(pane, snapshot),
      activate: () => activate(pane),
      isActive: () => model.active === pane,
      dialog: (...args) => dialog(pane, ...args),
      showError,
      dialogOpen: () => el("dialog").open,
      closeDialog: () => dialogFinish?.(null),
      runVisible: (action, args, onProgress) =>
        runVisible(pane, action, args, onProgress),
      clipboard: () => model.clipboard,
      copy: (cut) => model.copy(pane, cut),
      clearClipboard: () => {
        model.clipboard = null;
      },
      beginDrag: () => model.beginDrag(pane),
      isDragging: () => !!model.drag,
      endDrag: () => {
        model.drag = null;
        for (const entry of panes.values()) entry.controller?.dragEnded();
      },
      canDrop: (target, copy) => {
        const source = model.drag?.source;
        return (
          !!source &&
          model.validSource(source) &&
          (sameHome(source.home, model.snapshot(pane)?.home)
            ? !!dropPlan(source, target, model.snapshot(pane)?.home, copy)
            : true)
        );
      },
      drop: (token, target, copy) =>
        transfer(model.dragSource(token), pane, target, copy),
      paste: () => {
        const source = sourceForClipboard();
        return source ? transfer(source, pane, {}, source.copy) : undefined;
      },
      transferOther: (copy) =>
        transferToFolder(model.capture(pane), copy),
      portableControls: (project) => portable.controls(project),
      portableStatus: (project) => portable.summary(project),
      portableResume: (record) => runVisible("left", "portable_restore", { home: record.payload.home, anchor: record.payload.anchor, namespace: record.payload.namespace, target: record.payload.target || null, resume: record.id }),
      conclude: (record) => concludeTransfer(pane, record),
      resume: (record) =>
        runVisible(pane, "cross_store_resume", {
          home: record.payload.sourceHome || record.payload.home,
          id: record.id,
          targetHome: record.payload.targetHome,
        }),
      ready: (controller, boot) => {
        entry.controller = controller;
        entry.boot = boot;
        initialize().catch(showError);
      },
      storesChanged: () =>
        Promise.all(
          [...panes.values()].map((p) => p.controller?.reloadStores()),
        ),
    });
  },
});
async function concludeTransfer(pane, record) {
  const args = {
    home: record.payload.sourceHome || record.payload.home,
    targetHome: record.payload.targetHome,
    id: record.id,
  };
  const preview = await job("cross_store_conclude_plan", args);
  const confirmed = await dialog(
    pane,
    "전송 보존 후 안전 종료",
    (body) => {
      body.append(node("p", preview.warning));
      for (const root of preview.roots ?? [])
        body.append(
          node(
            "pre",
            `원본: ${root.source.path} (${root.source.exists ? "있음" : "없음"})\n대상: ${root.target.path} (${root.target.exists ? "있음" : "없음"})\n보관: ${root.hold.path} (${root.hold.exists ? "있음" : "없음"})\n${root.action === "restore-complete-hold" ? "완전한 보관본을 비어 있는 원래 위치로 복원" : "현재 위치에서 그대로 보존"}${root.sourceUnavailable ? "\n원래 위치의 파일을 사용할 수 없음 — 대상/보관 경로에서 확인 필요" : ""}`,
          ),
        );
      for (const t of preview.conversations ?? [])
        body.append(
          node(
            "p",
            `대화 ${t.id}: 원본 ${t.sourcePresent ? "있음" : "없음"} / 대상 ${t.targetPresent ? "있음" : "없음"}`,
          ),
        );
      body.append(
        node(
          "pre",
          `보관되는 임시 파일: ${preview.retainedStaging.path}\n대화 복구 사본: ${preview.recoveryPath}\n작업 기록: ${(preview.childJournals ?? []).map((j) => j.path).join("\n")}`,
        ),
      );
      return () => true;
    },
    "보존 후 안전 종료",
  );
  if (!confirmed) return;
  await runVisible(pane, "cross_store_conclude", {
    ...args,
    expectedRevision: preview.revision,
  });
}

async function initialize() {
  if (
    initializing ||
    panes.size !== 1 ||
    [...panes.values()].some((p) => !p.controller)
  )
    return;
  initializing = true;
  const primary = panes.get("left");
  el("version").textContent = primary.boot.version;
  showSplit(primary.boot.settings?.split_view === true);
  await invoke("ready");
  window.__TAURI__.event.listen("close-blocked", (event) =>
    showError(event.payload),
  );
  await primary.controller.startGlobals(primary.boot);
  const path = primary.boot.settings?.panes?.right?.destination;
  if (path) destination.navigate(path).catch((e) => setStatus(String(e)));
}
frames.left.src = "pane.html";
for (const [id, action] of [
  ["global-import", "import"],
  ["global-history", "history"],
  ["global-settings", "settings"],
])
  el(id).addEventListener("click", () => {
    const entry = panes.get(model.active);
    if (entry?.controller)
      Promise.resolve(entry.controller.action(action)).catch(showError);
  });
el("updates").addEventListener("click", () =>
  panes.get("left")?.controller?.action("updates").catch(showError),
);
el("transfer-move").addEventListener("click", () =>
  transferOther(false).catch(showError),
);
el("transfer-copy").addEventListener("click", () =>
  transferOther(true).catch(showError),
);
el("cancel").addEventListener("click", async () => {
  if (!visibleJob?.id) return;
  try {
    await invoke("cancel_job", { id: visibleJob.id });
    el("cancel").textContent = "취소 중";
    el("cancel").disabled = true;
  } catch (error) {
    await showError(error);
  }
});
document.addEventListener("keydown", (event) => {
  if (
    event.isComposing ||
    (event.key === "Enter" && event.target.tagName === "BUTTON")
  )
    return;
  const editing = event.target.matches(
    'input,textarea,select,[contenteditable="true"]',
  );
  const target = keyboardTarget(model.active, {
    editing,
    dialog: el("dialog").open,
  });
  if (target) panes.get(target)?.controller?.key(event);
});
let split = 50;
function resize(value) {
  split = Math.max(25, Math.min(75, value));
  el("explorers").style.setProperty("--split", split + "%");
  el("splitter").setAttribute("aria-valuenow", String(Math.round(split)));
  el("splitter").setAttribute(
    "aria-valuetext",
    "왼쪽 " + Math.round(split) + "%, 오른쪽 " + Math.round(100 - split) + "%",
  );
}
el("splitter").addEventListener("keydown", (event) => {
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  event.stopPropagation();
  resize(
    event.key === "Home"
      ? 25
      : event.key === "End"
        ? 75
        : split + (event.key === "ArrowLeft" ? -2 : 2),
  );
});
el("splitter").addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  el("splitter").setPointerCapture(event.pointerId);
});
el("splitter").addEventListener("pointermove", (event) => {
  if (!el("splitter").hasPointerCapture(event.pointerId)) return;
  const rect = el("explorers").getBoundingClientRect();
  resize(((event.clientX - rect.left) / rect.width) * 100);
});
el("splitter").addEventListener("pointerup", (event) => {
  if (el("splitter").hasPointerCapture(event.pointerId))
    el("splitter").releasePointerCapture(event.pointerId);
});
