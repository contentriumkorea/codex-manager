import {
  PaneModel,
  sameHome,
  normalizeHome,
  keyboardTarget,
} from "./pane-model.mjs";
import { dropPlan, formatBytes, duration } from "./explorer-model.mjs";
const el = (id) => document.getElementById(id);
const invoke = (name, args = {}) => window.__TAURI__.core.invoke(name, args);
const model = new PaneModel();
const panes = new Map();
const sizeJobs = new Map();
const frames = { left: el("left-frame"), right: el("right-frame") };
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
  model.active = pane;
  for (const id of ["left", "right"])
    el(id + "-pane").classList.toggle("active", id === pane);
  el("active-label").textContent =
    (pane === "left" ? "왼쪽" : "오른쪽") + " 탐색기 선택됨";
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
  if (!source || !model.validSource(source))
    throw new Error("원본 선택이 바뀌었습니다. 다시 선택하세요.");
  if (visibleJob) throw new Error("진행 중인 작업이 있습니다.");
  const destinationPane = model.snapshot(targetPane);
  if (!destinationPane?.home) throw new Error("대상 저장소를 연결하세요.");
  const destination = targetFor(targetPane, target);
  if (source.kind === "files" && !destination.projectId)
    throw new Error("파일을 받을 대상 프로젝트의 폴더를 선택하세요.");
  if (sameHome(source.home, destinationPane.home)) {
    if (copy && source.kind !== "files")
      throw new Error(
        "같은 저장소에서는 대화 이동·프로젝트 합치기를 사용하세요. 복사는 서로 다른 저장소에서 지원합니다.",
      );
    const action = dropPlan(source, destination, destinationPane.home, copy);
    if (!action) throw new Error("대상 프로젝트 또는 파일 폴더를 선택하세요.");
    await panes.get(targetPane).controller.performDrop(action);
    model.clipboard = null;
    return;
  }
  const ticket = model.ticket(source, targetPane);
  let plan = null,
    requestVersion = 0,
    previewVersion = -1;
  const answer = await dialog(
    source.pane,
    "반대편 저장소로 이동·복사",
    (body) => {
      const sequence = dialogSequence;
      body.append(
        node(
          "div",
          `원본 저장소: ${source.home}\n대상 저장소: ${destinationPane.home}\n${source.kind === "projects" ? "프로젝트" : source.kind === "files" ? "파일" : "대화"} ${source.ids.length}개\n${(source.names ?? source.ids).join("\n")}`,
          "summary-box",
        ),
      );
      const base = field("대상 부모 폴더", destination.directory);
      const operation = node("select");
      operation.setAttribute("aria-label", "전송 방식");
      for (const [value, label] of [
        ["move", "이동"],
        ["copy", "복사"],
      ]) {
        const option = node("option", label);
        option.value = value;
        operation.append(option);
      }
      operation.value = copy ? "copy" : "move";
      const project = node("select");
      project.setAttribute("aria-label", "대상 프로젝트");
      const fresh = node("option", "새 프로젝트로 가져오기");
      fresh.value = "";
      project.append(fresh);
      for (const p of destinationPane.projects ?? []) {
        const option = node("option", p.name);
        option.value = p.id;
        project.append(option);
      }
      project.value = destination.projectId || "";
      const mapping = node(
        "div",
        "대상 경로를 확인한 뒤 미리보기를 생성하세요.",
        "summary-box transfer-mapping",
      );
      mapping.setAttribute("aria-live", "polite");
      const warnings = node("div", "", "transfer-warning");
      const preview = node("button", "미리보기 다시 계산");
      preview.type = "button";
      const invalidate = () => {
        requestVersion++;
        plan = null;
        el("dialog-ok").disabled = true;
        mapping.textContent = "변경된 조건의 미리보기를 다시 계산하세요.";
        warnings.textContent = "";
      };
      for (const input of [base.input, operation, project])
        input.addEventListener("input", invalidate);
      const regenerate = async () => {
        if (!model.valid(ticket))
          throw new Error(
            "탐색기 상태가 바뀌었습니다. 창을 닫고 다시 전송하세요.",
          );
        const version = requestVersion;
        plan = null;
        preview.disabled = true;
        el("dialog-ok").disabled = true;
        mapping.textContent = "파일과 대화 연결을 확인하고 있습니다.";
        try {
          const result = await job("cross_store_plan", {
            home: source.home,
            targetHome: destinationPane.home,
            kind: source.kind,
            ids: source.ids,
            targetProject: project.value || null,
            destination: base.input.value.trim(),
            copy: operation.value === "copy",
          });
          if (
            version !== requestVersion ||
            sequence !== dialogSequence ||
            !model.valid(ticket) ||
            !el("dialog").open
          )
            return;
          plan = result;
          previewVersion = version;
          mapping.textContent =
            `프로젝트 ${result.projects?.length ?? 0}개 · 대화 ${result.threads?.length ?? 0}개\n` +
            (result.roots ?? [])
              .map(
                (root) =>
                  `${root.source}\n→ ${root.destination}${root.retainSource ? "\n  원본 파일 유지" : ""}`,
              )
              .join("\n\n");
          warnings.textContent = (result.warnings ?? []).join("\n");
          el("dialog-ok").disabled = false;
        } finally {
          preview.disabled = false;
        }
      };
      preview.addEventListener("click", () =>
        regenerate().catch((error) => {
          mapping.textContent = String(error);
          setStatus(String(error));
        }),
      );
      body.append(
        base.box,
        node("p", "전송 방식"),
        operation,
        node("p", "대상 프로젝트"),
        project,
        preview,
        mapping,
        warnings,
        node(
          "p",
          "이동은 대상 검증 후 원본을 정리합니다. 중단된 전송은 작업 내역에서 계속할 수 있습니다.",
        ),
      );
      el("dialog-ok").disabled = true;
      if (base.input.value)
        queueMicrotask(() =>
          regenerate().catch((error) => {
            mapping.textContent = String(error);
          }),
        );
      return () => {
        if (!plan || previewVersion !== requestVersion || !model.valid(ticket))
          throw new Error(
            "조건이 바뀌었거나 미리보기가 없습니다. 미리보기를 다시 계산하세요.",
          );
        return plan;
      };
    },
    "확인한 내용으로 전송",
  );
  if (!answer) return;
  if (!model.valid(ticket))
    throw new Error("탐색기 상태가 바뀌었습니다. 다시 미리보기를 확인하세요.");
  await job("pane_preferences", {
    pane: targetPane,
    destination: answer.request.destination,
  });
  if (!model.valid(ticket))
    throw new Error("탐색기 상태가 바뀌었습니다. 다시 미리보기를 확인하세요.");
  panes.get(targetPane).controller.setDestination(answer.request.destination);
  try {
    await runVisible(source.pane, "cross_store_transfer", {
      ...answer.request,
      expectedRevision: answer.sourceRevision,
      targetRevision: answer.targetRevision,
    });
    model.clipboard = null;
  } finally {
    for (const entry of panes.values()) entry.controller.render();
  }
}
async function transferOther(copy = false) {
  const source = model.capture(model.active);
  if (!source) throw new Error("옮길 항목을 먼저 선택하세요.");
  return transfer(source, model.active === "left" ? "right" : "left", {}, copy);
}
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
        transfer(
          model.capture(pane),
          pane === "left" ? "right" : "left",
          {},
          copy,
        ),
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
async function initialize() {
  if (
    initializing ||
    panes.size !== 2 ||
    [...panes.values()].some((p) => !p.controller)
  )
    return;
  initializing = true;
  const primary = panes.get("left");
  el("version").textContent = primary.boot.version;
  await invoke("ready");
  window.__TAURI__.event.listen("close-blocked", (event) =>
    showError(event.payload),
  );
  await primary.controller.startGlobals(primary.boot);
}
for (const pane of ["left", "right"]) frames[pane].src = "pane.html";
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
