import { visibleProjects } from "./view-model.mjs";
import {
  sortedRows,
  selectRange,
  dropPlan,
  formatBytes,
  duration,
  safeFolderName,
  joinPath,
} from "./explorer-model.mjs";
const el = (id) => document.getElementById(id);
const invoke = (name, args = {}) => window.__TAURI__.core.invoke(name, args);
const state = {
  home: "",
  catalog: { projects: [], unassigned: [], unresolved: [] },
  mode: "projects",
  project: null,
  selected: new Set(),
  anchor: null,
  sort: { key: "name", direction: "asc" },
  clipboard: null,
  drag: null,
  directory: "",
  files: [],
  sizes: {},
  busy: false,
  job: null,
  history: [],
  refresh: 0,
};
const node = (tag, value, cls) => {
  const n = document.createElement(tag);
  if (value !== undefined) n.textContent = value;
  if (cls) n.className = cls;
  return n;
};
const button = (label, callback, cls) => {
  const b = node("button", label, cls);
  b.type = "button";
  b.addEventListener("click", () =>
    Promise.resolve(callback()).catch(showError),
  );
  return b;
};
const titleOf = (t) => t.name || t.id || "제목 없는 대화";
const allThreads = () => [
  ...state.catalog.projects.flatMap((g) => g.threads),
  ...state.catalog.unassigned,
  ...state.catalog.unresolved,
];
const group = (id) => state.catalog.projects.find((g) => g.project.id === id);
const currentGroup = () => group(state.project);
const selectedRows = () => rows().filter((row) => state.selected.has(row.id));
function setStatus(text) {
  el("status").textContent = text;
}
function field(label, value = "", type = "text") {
  const box = node("label", undefined, "field");
  box.append(node("span", label));
  const input = node("input");
  input.type = type;
  input.value = value;
  input.setAttribute("aria-label", label);
  box.append(input);
  return { box, input };
}
function check(label, checked = false) {
  const box = node("label", undefined, "check-label");
  const input = node("input");
  input.type = "checkbox";
  input.checked = checked;
  box.append(input, node("span", label));
  return { box, input };
}
function pathField(label, value = "") {
  const f = field(label, value);
  f.input.remove();
  const row = node("div", undefined, "path-field");
  row.append(
    f.input,
    button("찾기", async () => {
      const path = await invoke("choose_folder", {
        initial: f.input.value || null,
      });
      if (path) {
        f.input.value = path;
        f.input.dispatchEvent(new Event("input", { bubbles: true }));
      }
    }),
  );
  row.lastChild.textContent = "찾기";
  f.box.append(row);
  return f;
}
function dialog(title, build, ok = "확인", danger = false) {
  if (el("dialog").open) el("dialog").close();
  el("dialog-title").textContent = title;
  el("dialog-content").replaceChildren();
  el("dialog-error").hidden = true;
  el("dialog-ok").textContent = ok;
  el("dialog-ok").className = danger ? "danger" : "primary";
  el("dialog-ok").hidden = false;
  el("dialog-cancel").textContent = "취소";
  const collect = build(el("dialog-content"));
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      el("dialog").onclose = null;
      el("dialog").close();
      resolve(value);
    };
    el("dialog").onclose = () => finish(null);
    el("dialog-cancel").onclick = () => finish(null);
    el("dialog-close").onclick = () => finish(null);
    el("dialog-form").onsubmit = async (event) => {
      event.preventDefault();
      el("dialog-error").hidden = true;
      try {
        const value = await collect();
        finish(value ?? true);
      } catch (error) {
        el("dialog-error").textContent = String(error);
        el("dialog-error").hidden = false;
      }
    };
    el("dialog").showModal();
  });
}
function showError(error) {
  setStatus(String(error).split("\n")[0]);
  return dialog(
    "작업을 완료하지 못했습니다.",
    (body) => {
      body.append(node("div", String(error), "summary-box"));
      return () => true;
    },
    "닫기",
  );
}
async function confirm(title, description, danger = false) {
  return Boolean(
    await dialog(
      title,
      (body) => {
        body.append(node("div", description, "summary-box"));
        return () => true;
      },
      danger ? "삭제" : "진행",
      danger,
    ),
  );
}
function projectRows() {
  return visibleProjects(state.catalog, el("search").value, "asc").map((g) => ({
    id: g.project.id,
    name: g.project.name,
    path: g.project.roots.map((r) => r.path).join(" · "),
    count: g.threads.length,
    size: state.sizes[g.project.id]?.error
      ? undefined
      : state.sizes[g.project.id]?.bytes,
    project: g.project,
    threads: g.threads,
  }));
}
function rows() {
  let data;
  if (state.mode === "projects") data = projectRows();
  else if (state.mode === "files") data = state.files;
  else if (state.mode === "history")
    data = state.history.map((r) => ({
      id: r.id,
      name: historyLabel(r),
      state: r.state,
      createdAt: r.createdAt,
      record: r,
    }));
  else {
    const threads =
      state.mode === "unassigned"
        ? [...state.catalog.unassigned, ...state.catalog.unresolved]
        : (currentGroup()?.threads ?? []);
    data = threads.map((t) => ({
      ...t,
      name: titleOf(t),
      updatedAt: t.updatedAt ?? 0,
      path: t.cwd,
      status: t.archived
        ? "보관됨"
        : t.hasUserEvent === false
          ? "빈 대화"
          : t.internal
            ? "내부 대화"
            : "대화",
    }));
  }
  const term = el("search").value.trim().toLocaleLowerCase("ko-KR");
  if (state.mode !== "projects")
    data = data.filter((r) =>
      [r.name, r.path, r.state, r.createdAt].some((v) =>
        String(v ?? "")
          .toLocaleLowerCase("ko-KR")
          .includes(term),
      ),
    );
  return sortedRows(data, state.sort.key, state.sort.direction);
}
function historyLabel(r) {
  const p = r.payload;
  const labels = {
    "rust-backup": "프로젝트 백업",
    "rust-legacy": "프로젝트 연결 정리",
    "rust-import": "백업 가져오기",
    "rust-project-edit": "프로젝트 변경",
    "rust-thread-names": "대화 제목 변경",
    "rust-assign": "대화 이동",
    "rust-merge": "프로젝트 합치기",
    "rust-delete": p.permanent ? "완전 삭제" : "삭제",
    "rust-files":
      "파일 " +
      ({ copy: "복사", move: "이동", rename: "이름 변경", delete: "삭제" }[
        p.action
      ] ?? "작업"),
    "rust-transfer": "프로젝트 폴더 이동",
  };
  return (
    p.label ||
    labels[p.kind] ||
    {
      connections: "프로젝트 연결 변경",
      "management-delete": "프로젝트·대화 삭제",
      "rename-thread": "대화 제목 변경",
      "rename-threads": "대화 제목 변경",
      "file-edit": "파일 작업",
    }[p.kind] ||
    p.kind
  );
}
function stateLabel(r) {
  if (r.recovered) return "되돌림";
  return (
    {
      completed: "완료",
      cancelled: "취소",
      needs_recovery: "복구 필요",
      planned: "준비",
      copying: "복사 중",
      relinking: "연결 변경 중",
    }[r.state] ?? "중단됨 · 확인 필요"
  );
}
function columns() {
  if (state.mode === "projects")
    return [
      ["name", "이름"],
      ["path", "연결 폴더"],
      ["count", "대화"],
      ["size", "용량"],
    ];
  if (state.mode === "files")
    return [
      ["name", "이름"],
      ["updatedAt", "수정한 날짜"],
      ["directory", "종류"],
      ["size", "크기"],
    ];
  if (state.mode === "history")
    return [
      ["name", "작업"],
      ["createdAt", "시간"],
      ["state", "상태"],
    ];
  return [
    ["name", "대화 제목"],
    ["path", "작업 폴더"],
    ["updatedAt", "최근 변경"],
    ["status", "상태"],
  ];
}
function date(value) {
  if (!value) return "—";
  const d = new Date(value * 1000);
  return Number.isNaN(d.getTime()) ? "—" : d.toLocaleDateString("ko-KR");
}
function cell(row, key) {
  if (key === "size") {
    if (state.mode === "projects" && state.sizes[row.id]?.error)
      return "확인 필요";
    return formatBytes(row.size);
  }
  if (key === "updatedAt") return date(row.updatedAt);
  if (key === "directory")
    return row.linked ? "연결 항목" : row.directory ? "폴더" : "파일";
  if (key === "state") return stateLabel(row.record);
  return String(row[key] ?? "—");
}
function render() {
  const data = rows();
  const ids = data.map((r) => r.id);
  state.selected = new Set(
    [...state.selected].filter((id) => ids.includes(id)),
  );
  el("columns").replaceChildren();
  for (const [key, label] of columns()) {
    const th = node("th");
    th.setAttribute("scope", "col");
    if (state.sort.key === key)
      th.setAttribute(
        "aria-sort",
        state.sort.direction === "asc" ? "ascending" : "descending",
      );
    th.append(
      button(
        label +
          (state.sort.key === key
            ? state.sort.direction === "asc"
              ? " ↑"
              : " ↓"
            : ""),
        () => {
          state.sort = {
            key,
            direction:
              state.sort.key === key && state.sort.direction === "asc"
                ? "desc"
                : "asc",
          };
          render();
        },
      ),
    );
    el("columns").append(th);
  }
  el("rows").replaceChildren();
  for (const row of data) {
    const tr = node("tr");
    tr.tabIndex = -1;
    tr.dataset.id = row.id;
    tr.setAttribute("aria-selected", String(state.selected.has(row.id)));
    if (state.clipboard?.cut && state.clipboard.ids.includes(row.id))
      tr.classList.add("cut");
    for (const [key] of columns()) {
      const td = node("td");
      if (key === "name") {
        const wrap = node("div", undefined, "row-name");
        wrap.append(
          node(
            "span",
            state.mode === "projects" || row.directory
              ? "▱"
              : state.mode === "history"
                ? "◷"
                : "▤",
            "row-icon",
          ),
          node("span", row.name),
        );
        const label = node("button", undefined, "row-label");
        label.type = "button";
        label.setAttribute("aria-label", row.name);
        label.append(wrap);
        td.append(label);
      } else td.textContent = cell(row, key);
      td.title = cell(row, key);
      tr.append(td);
    }
    tr.addEventListener("click", (event) => {
      state.selected = selectRange(
        ids,
        state.selected,
        row.id,
        state.anchor,
        event,
      );
      if (!event.shiftKey) state.anchor = row.id;
      render();
    });
    tr.addEventListener("dblclick", () => openRow(row));
    tr.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      if (!state.selected.has(row.id)) {
        state.selected = new Set([row.id]);
        state.anchor = row.id;
        render();
      }
      context(event.clientX, event.clientY);
    });
    tr.draggable = state.mode !== "history";
    tr.addEventListener("dragstart", (event) => {
      if (!state.selected.has(row.id)) state.selected = new Set([row.id]);
      state.drag = { kind: kind(), home: state.home, ids: [...state.selected] };
      event.dataTransfer.setData(
        "application/x-codex-manager",
        JSON.stringify(state.drag),
      );
      event.dataTransfer.effectAllowed = "copyMove";
    });
    tr.addEventListener("dragend", () => {
      state.drag = null;
      document
        .querySelectorAll(".drop-target")
        .forEach((n) => n.classList.remove("drop-target"));
    });
    if (state.mode === "projects")
      dropTarget(tr, {
        projectId: row.id,
        directory: row.project.roots[0]?.path,
      });
    if (state.mode === "files" && row.directory && !row.linked)
      dropTarget(tr, { directory: row.path });
    el("rows").append(tr);
  }
  el("empty").hidden = data.length > 0;
  el("table").setAttribute(
    "aria-label",
    state.mode === "projects"
      ? "프로젝트 목록"
      : state.mode === "files"
        ? "파일 목록"
        : state.mode === "history"
          ? "작업 내역"
          : "대화 목록",
  );
  el("title").textContent =
    state.mode === "projects"
      ? "모든 프로젝트"
      : state.mode === "unassigned"
        ? "프로젝트 미지정"
        : state.mode === "history"
          ? "작업 내역·복구"
          : currentGroup()?.project.name || "프로젝트";
  el("tabs").hidden = !["threads", "files"].includes(state.mode);
  el("tab-threads").classList.toggle("active", state.mode === "threads");
  el("tab-files").classList.toggle("active", state.mode === "files");
  el("back").hidden = !["threads", "files"].includes(state.mode);
  el("file-path-bar").hidden = state.mode !== "files";
  el("backup").hidden = state.mode === "history" || state.mode === "unassigned";
  el("rename").hidden = state.mode === "history";
  el("move").hidden = state.mode === "history";
  el("delete").hidden = state.mode === "history";
  el("rename").disabled = state.busy || !state.selected.size;
  el("delete").disabled = state.busy || !state.selected.size;
  el("move").disabled = state.busy || !state.selected.size;
  el("backup").disabled =
    state.busy || (state.mode === "projects" && !state.selected.size);
  el("import").disabled = state.busy;
  el("project-count").textContent = state.catalog.projects.length;
  el("unassigned-count").textContent =
    state.catalog.unassigned.length + state.catalog.unresolved.length;
  el("nav-projects").classList.toggle("active", state.mode === "projects");
  el("nav-unassigned").classList.toggle("active", state.mode === "unassigned");
  el("nav-history").classList.toggle("active", state.mode === "history");
  el("subtitle").textContent =
    state.mode === "files"
      ? state.directory
      : state.mode === "history"
        ? "작업 결과를 확인하고 중단된 작업을 복구하세요."
        : state.mode === "unassigned"
          ? "명시적인 소속이 없는 대화와 연결 확인이 필요한 대화입니다."
          : state.mode === "projects"
            ? "프로젝트를 두 번 클릭해 대화와 파일을 열어보세요."
            : `${currentGroup()?.threads.length ?? 0}개 대화 · ${currentGroup()?.project.roots.length ?? 0}개 폴더`;
  renderDetails();
  if (!state.busy)
    setStatus(
      `${data.length}개 항목${state.selected.size ? " · " + state.selected.size + "개 선택" : ""}`,
    );
}
function renderNav() {
  el("project-nav").replaceChildren();
  for (const g of state.catalog.projects) {
    const b = button("▱  " + g.project.name, () =>
      activateProject(g.project.id),
    );
    b.title = g.project.roots.map((r) => r.path).join("\n");
    b.classList.toggle(
      "active",
      state.project === g.project.id &&
        ["threads", "files"].includes(state.mode),
    );
    dropTarget(b, {
      projectId: g.project.id,
      directory: g.project.roots[0]?.path,
    });
    el("project-nav").append(b);
  }
}
function renderDetails() {
  const body = el("details");
  body.replaceChildren();
  const selected = selectedRows();
  if (!selected.length) {
    body.append(
      node("div", state.mode === "files" ? "▱" : "▦", "detail-icon"),
      node(
        "h2",
        state.mode === "history" ? "변경 이력과 복구" : "선택한 항목의 정보",
      ),
      node(
        "p",
        state.mode === "history"
          ? "작업을 선택하면 결과와 복구 방법을 볼 수 있습니다."
          : "Ctrl·Shift로 여러 항목을 선택하고 다른 프로젝트나 폴더로 끌어 옮길 수 있습니다.",
      ),
      node(
        "div",
        "F2 이름 변경 · Delete 삭제\nCtrl+C 복사 · Ctrl+X 잘라내기\nCtrl+V 붙여넣기 · Ctrl+Z 되돌리기",
        "tip",
      ),
    );
    return;
  }
  const first = selected[0];
  body.append(
    node(
      "div",
      selected.length > 1 ? "▦" : state.mode === "files" ? "▱" : "▤",
      "detail-icon",
    ),
    node(
      "h2",
      selected.length > 1 ? selected.length + "개 항목 선택" : first.name,
    ),
  );
  const dl = node("dl");
  const info = (label, value) => {
    dl.append(node("dt", label), node("dd", value));
  };
  if (state.mode === "projects") {
    info("연결된 대화", selected.reduce((s, r) => s + r.count, 0) + "개");
    for (const r of selected)
      info(
        r.name,
        r.project.roots.map((root) => root.path).join("\n") ||
          "연결된 폴더 없음",
      );
    body.append(
      button("대화 열기", () => activateProject(first.id)),
      button("파일 열기", () => activateProject(first.id, "files")),
    );
  } else if (state.mode === "history") {
    const r = first.record;
    info("처리 결과", stateLabel(r));
    info("저장소", r.payload.home ?? "");
    if (r.lastEvent?.payload?.error)
      info("확인할 내용", r.lastEvent.payload.error);
    if (!r.recovered && r.payload.permanent !== true) {
      if (r.payload.kind === "rust-import" && r.state !== "completed")
        body.append(
          button("이어서 가져오기", () => resumeImport(r), "primary"),
        );
      else if (
        !["rust-backup", "batch-report", "rust-legacy"].includes(r.payload.kind)
      )
        body.append(button("이전 상태로 복구", () => undo(r.id), "primary"));
    }
    if (
      r.payload.kind === "rust-legacy" &&
      !["completed", "cancelled"].includes(r.state)
    )
      body.append(
        button(
          "프로젝트 연결 정리 계속",
          () => runJob("resume_legacy", {}),
          "primary",
        ),
      );
    if (
      r.payload.kind !== "rust-legacy" &&
      !["completed", "cancelled"].includes(r.state)
    )
      body.append(
        button("현재 상태를 유지하고 작업 종료", async () => {
          const yes = await dialog(
            "미완료 작업 종료",
            (body) => {
              body.append(
                node(
                  "p",
                  "현재 파일과 대화를 그대로 보존하고 미완료 상태만 종료합니다. 이미 처리된 삭제 항목은 복구되지 않습니다.",
                ),
              );
              return () => true;
            },
            "현재 상태 유지",
          );
          if (yes) await runJob("keep_current", { id: r.id });
        }),
      );
    if (r.payload.destination) info("백업 위치", r.payload.destination);
    if (r.payload.recovery) info("복구 사본", r.payload.recovery);
  } else if (state.mode === "files") {
    info("위치", first.path);
    info("크기", first.directory ? "폴더" : formatBytes(first.size));
    if (first.linked)
      info("연결 항목", "안전을 위해 링크는 변경하지 않습니다.");
  } else {
    info("작업 폴더", first.cwd ?? "");
    info("대화 ID", first.id);
    info("상태", first.status);
    body.append(button("대화 내용 보기", () => viewTranscript(first)));
  }
  body.append(dl);
}
function kind() {
  return state.mode === "projects"
    ? "projects"
    : state.mode === "files"
      ? "files"
      : "threads";
}
function dropTarget(element, target) {
  const plan = (event) =>
    dropPlan(state.drag, target, state.home, event.ctrlKey);
  element.addEventListener("dragover", (event) => {
    if (state.busy || !plan(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect =
      state.drag.kind === "files" && event.ctrlKey ? "copy" : "move";
    element.classList.add("drop-target");
  });
  element.addEventListener("dragleave", () =>
    element.classList.remove("drop-target"),
  );
  element.addEventListener("drop", (event) => {
    event.preventDefault();
    event.stopPropagation();
    element.classList.remove("drop-target");
    const action = plan(event);
    state.drag = null;
    if (action) performDrop(action).catch(showError);
  });
}
async function performDrop(plan) {
  if (plan.action === "merge") {
    const target = group(plan.target);
    if (
      await confirm(
        "프로젝트 합치기",
        `${plan.ids.map((id) => group(id)?.project.name ?? id).join("\n")}\n→ ${target?.project.name}\n\n대상 이름을 유지합니다. 대화와 연결 폴더를 합치고 원본 프로젝트 항목을 제거합니다. 파일 위치는 유지합니다.`,
      )
    )
      await runJob("merge", plan);
  } else if (plan.action === "move_threads") {
    if (
      await confirm(
        "대화 옮기기",
        `대화 ${plan.ids.length}개 → ${group(plan.target)?.project.name}\n대화의 프로젝트 소속을 변경합니다. 작업 파일 위치는 유지합니다.`,
      )
    )
      await runJob("move_threads", plan);
  } else if (plan.action === "files") {
    if (
      plan.kind === "copy" ||
      (await confirm(
        "파일 이동",
        `${plan.sources.length}개 항목\n→ ${plan.destination}`,
      ))
    )
      await runJob("files", plan);
  }
  state.clipboard = null;
  render();
}
async function activateProject(id, mode = "threads") {
  state.project = id;
  state.mode = mode;
  state.selected.clear();
  state.anchor = null;
  el("search").value = "";
  state.sort = { key: "name", direction: "asc" };
  renderNav();
  if (mode === "files") {
    const roots = currentGroup()?.project.roots ?? [];
    el("root-picker").replaceChildren(
      ...roots.map((root) => {
        const o = node("option", root.path);
        o.value = root.path;
        return o;
      }),
    );
    await navigate(roots[0]?.path ?? "");
  } else render();
}
async function navigate(path) {
  state.mode = "files";
  state.directory = path;
  el("file-path").value = path;
  state.selected.clear();
  if (!path) {
    state.files = [];
    render();
    return;
  }
  try {
    state.files = await invoke("file_list", {
      home: state.home,
      projects: [state.project],
      directory: path,
    });
    render();
  } catch (error) {
    state.files = [];
    render();
    await showError(error);
  }
}
function openRow(row) {
  if (state.mode === "projects") activateProject(row.id).catch(showError);
  else if (state.mode === "files" && row.directory && !row.linked)
    navigate(row.path);
  else if (state.mode === "history") renderDetails();
  else if (["threads", "unassigned"].includes(state.mode))
    viewTranscript(row).catch(showError);
}
async function viewTranscript(row) {
  setStatus("대화 내용을 읽고 있습니다.");
  const result = await invoke("transcript", { home: state.home, id: row.id });
  await dialog(
    titleOf(result.thread),
    (body) => {
      body.append(
        node(
          "p",
          result.total > 200
            ? `총 ${result.total}개 메시지 중 최근 200개를 표시합니다.`
            : "저장된 대화 내용입니다.",
        ),
      );
      for (const message of result.messages) {
        const box = node("div", undefined, "message " + message.role);
        box.append(
          node("small", message.role === "user" ? "나" : "Codex"),
          node("div", message.text),
        );
        body.append(box);
      }
      return () => true;
    },
    "닫기",
  );
}
async function refresh(withSizes = true) {
  const generation = ++state.refresh;
  try {
    const catalog = await invoke("catalog", { home: state.home });
    if (generation !== state.refresh) return;
    state.catalog = catalog;
    if (state.project && !group(state.project)) {
      state.mode = "projects";
      state.project = null;
    }
    renderNav();
    if (state.mode === "history") await loadHistory();
    else if (state.mode === "files") await navigate(state.directory);
    else render();
    if (withSizes) autoSizes(state.home, generation);
  } catch (error) {
    state.catalog = { projects: [], unassigned: [], unresolved: [] };
    renderNav();
    render();
    setStatus(String(error));
    el("notice").textContent = String(error);
    el("notice").hidden = false;
  }
}
async function autoSizes(home, generation) {
  try {
    el("size-status").textContent = "용량 계산 중";
    const id = await invoke("start_job", { action: "sizes", args: { home } });
    const result = await pollJob(id, false);
    if (generation !== state.refresh || home !== state.home) return;
    state.sizes = result;
    el("size-status").textContent = "용량 계산 완료";
    render();
  } catch (error) {
    el("size-status").textContent = String(error).split("\n")[0];
  }
}
async function pollJob(id, show = true, onProgress) {
  for (;;) {
    const p = await invoke("job_status", { id });
    if (onProgress) onProgress(p);
    if (show) renderProgress(p);
    if (p.state === "completed") return p.result;
    if (p.state === "failed" || p.state === "cancelled")
      throw new Error(p.error || "작업을 완료하지 못했습니다.");
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}
function renderProgress(p) {
  el("progress").hidden = false;
  el("progress-phase").textContent = p.phase;
  el("progress-path").textContent = p.path ?? "";
  const percent =
    p.total > 0 ? Math.min(99, Math.floor((p.done / p.total) * 100)) : null;
  el("progress-percent").textContent =
    p.state === "completed"
      ? "완료"
      : percent === null
        ? "작업 중"
        : percent + "%";
  if (p.state === "completed") el("progress-bar").value = 100;
  else if (percent === null) el("progress-bar").removeAttribute("value");
  else el("progress-bar").value = percent;
  el("progress-stats").textContent =
    `경과 ${duration(p.elapsed)}${p.total > 0 ? " · " + (p.unit === "items" ? p.done + " / " + p.total + "개" : formatBytes(p.done) + " / " + formatBytes(p.total)) : ""}${p.speed > 0 ? " · " + formatBytes(p.speed) + "/초" : ""}${p.remaining > 0 && p.speed > 0 ? " · 약 " + duration(p.remaining) + " 남음" : ""}`;
  el("cancel").disabled = p.state !== "running";
}
async function runJob(action, args, onProgress) {
  if (state.busy) throw new Error("진행 중인 작업이 있습니다.");
  state.busy = true;
  state.job = null;
  el("cancel").textContent = "취소";
  render();
  try {
    const id = await invoke("start_job", {
      action,
      args: {
        ...args,
        home: state.home,
        expectedRevision: state.catalog.revision,
      },
    });
    state.job = id;
    const result = await pollJob(id, true, onProgress);
    state.busy = false;
    state.job = null;
    setStatus("작업을 완료했습니다.");
    await refresh(false);
    return result;
  } catch (error) {
    state.busy = false;
    state.job = null;
    el("progress-phase").textContent = "작업 결과 확인";
    setStatus(String(error));
    await refresh(false);
    throw error;
  } finally {
    render();
  }
}
async function newProject() {
  const answer = await dialog(
    "프로젝트 만들기",
    (body) => {
      const name = field("프로젝트 이름");
      const root = pathField("연결할 작업 폴더");
      body.append(name.box, root.box);
      return () => {
        if (!name.input.value.trim())
          throw new Error("프로젝트 이름을 입력하세요.");
        return {
          name: name.input.value.trim(),
          roots: root.input.value ? [root.input.value] : [],
        };
      };
    },
    "만들기",
  );
  if (answer) await runJob("create", answer);
}
async function rename() {
  const chosen = selectedRows();
  if (!chosen.length) return;
  if (state.mode === "files") {
    const answer = await dialog("파일 이름 변경", (body) => {
      if (chosen.length !== 1)
        throw new Error("파일 이름은 하나씩 변경하세요.");
      const name = field("새 이름", chosen[0].name);
      body.append(name.box);
      return () => ({ name: name.input.value.trim() });
    });
    if (answer)
      await runJob("files", {
        kind: "rename",
        sources: [chosen[0].path],
        destination: state.directory,
        ...answer,
      });
    return;
  }
  if (chosen.length === 1) {
    const answer = await dialog(
      state.mode === "projects" ? "프로젝트 이름 변경" : "대화 제목 변경",
      (body) => {
        const name = field("새 이름", chosen[0].name);
        body.append(name.box);
        return () => {
          if (!name.input.value.trim()) throw new Error("이름을 입력하세요.");
          return name.input.value.trim();
        };
      },
    );
    if (answer)
      await runJob(
        state.mode === "projects" ? "rename_project" : "rename_threads",
        state.mode === "projects"
          ? { id: chosen[0].id, name: answer }
          : { ids: [chosen[0].id], names: [answer] },
      );
  } else if (state.mode !== "projects") {
    const answer = await dialog(`대화 ${chosen.length}개 제목 변경`, (body) => {
      const find = field("찾을 글자");
      const replacement = field("바꿀 글자");
      const prefix = field("앞에 붙일 글자");
      const suffix = field("뒤에 붙일 글자");
      const preview = node("div", undefined, "summary-box");
      const names = () =>
        chosen.map((row) =>
          (
            prefix.input.value +
            (find.input.value
              ? row.name.split(find.input.value).join(replacement.input.value)
              : row.name) +
            suffix.input.value
          ).trim(),
        );
      for (const f of [find, replacement, prefix, suffix]) {
        f.input.addEventListener(
          "input",
          () => (preview.textContent = names().join("\n")),
        );
        body.append(f.box);
      }
      preview.textContent = names().join("\n");
      body.append(preview);
      return () => ({ ids: chosen.map((r) => r.id), names: names() });
    });
    if (answer) await runJob("rename_threads", answer);
  } else throw new Error("프로젝트 이름은 하나씩 변경하세요.");
}
async function deleteItems(permanent = false) {
  const selected = selectedRows();
  if (!selected.length) return;
  if (state.mode === "files") {
    if (
      await confirm(
        "파일 삭제",
        `${selected.map((r) => r.name).join("\n")}\n\n복구 보관소로 옮깁니다. Ctrl+Z로 되돌릴 수 있으며 용량은 바로 확보되지 않습니다.`,
        true,
      )
    )
      await runJob("files", {
        kind: "delete",
        sources: selected.map((r) => r.path),
        destination: state.directory,
      });
    return;
  }
  const answer = await dialog(
    "선택한 항목 삭제",
    (body) => {
      body.append(
        node("div", selected.map((r) => r.name).join("\n"), "summary-box"),
      );
      const forever = check(
        "완전 삭제 · 복구 사본을 만들지 않으며 되돌릴 수 없습니다.",
        permanent,
      );
      body.append(
        forever.box,
        node(
          "p",
          "프로젝트를 삭제하면 연결된 대화와 내부 자식 대화도 처리됩니다. 작업 폴더와 파일은 유지합니다.",
          "warning",
        ),
      );
      return () => ({ permanent: forever.input.checked });
    },
    "삭제",
    true,
  );
  if (answer)
    await runJob("delete", {
      projects: state.mode === "projects" ? selected.map((r) => r.id) : [],
      threads: state.mode === "projects" ? [] : selected.map((r) => r.id),
      ...answer,
    });
}
async function backup() {
  const ids =
    state.mode === "projects"
      ? [...state.selected]
      : state.project
        ? [state.project]
        : [];
  if (!ids.length) return;
  const answer = await dialog(
    "프로젝트 백업",
    (body) => {
      body.append(
        node(
          "div",
          ids.map((id) => group(id)?.project.name ?? id).join("\n"),
          "summary-box",
        ),
      );
      const path = pathField("백업을 보관할 상위 폴더");
      body.append(
        path.box,
        node(
          "p",
          "프로젝트마다 새 백업 폴더를 만들고 파일과 대화를 함께 저장합니다. 외장하드를 선택하면 다른 컴퓨터에서도 가져올 수 있습니다.",
        ),
      );
      return () => {
        if (!path.input.value) throw new Error("백업 대상 폴더를 선택하세요.");
        return { ids, destination: path.input.value };
      };
    },
    "백업 시작",
  );
  if (answer) {
    const result = await runJob("backup", answer);
    setStatus(result.backups.map((b) => b.path).join(" · "));
    if (result.backups.some((b) => b.dependencies?.length))
      await dialog(
        "백업의 추가 연결을 확인하세요",
        (body) => {
          body.append(
            node(
              "p",
              "일부 백업에 외부 작업 폴더·첨부·다른 프로젝트의 자식 대화가 있어 자동 이식에 제한이 있습니다. 원본을 삭제하지 말고 연결을 확인한 뒤 다시 백업하세요.",
            ),
          );
          return () => true;
        },
        "확인",
      );
  }
}
async function moveDialog() {
  const chosen = selectedRows();
  if (!chosen.length) return;
  if (state.mode === "files") {
    const answer = await dialog("파일 이동", (body) => {
      const path = pathField("대상 폴더", state.directory);
      body.append(path.box);
      return () => ({
        action: "files",
        kind: "move",
        sources: chosen.map((r) => r.path),
        destination: path.input.value,
      });
    });
    if (answer) await performDrop(answer);
    return;
  }
  const projectMode = state.mode === "projects";
  const ids = chosen.map((r) => r.id);
  const answer = await dialog(
    projectMode ? "프로젝트 이동·합치기" : "대화 이동",
    (body) => {
      body.append(
        node("div", chosen.map((r) => r.name).join("\n"), "summary-box"),
      );
      const label = node("label", undefined, "field");
      label.append(node("span", "대상 프로젝트"));
      const picker = node("select");
      picker.setAttribute("aria-label", "대상 프로젝트");
      for (const g of state.catalog.projects.filter(
        (g) => !projectMode || !ids.includes(g.project.id),
      )) {
        const o = node("option", g.project.name);
        o.value = g.project.id;
        picker.append(o);
      }
      label.append(picker);
      body.append(label);
      if (projectMode && chosen.length === 1)
        body.append(
          button("다른 폴더로 프로젝트 이동", async () => {
            el("dialog").close();
            await transferDialog(chosen[0]);
          }),
        );
      return () => {
        if (!picker.value) throw new Error("대상 프로젝트를 선택하세요.");
        return {
          action: projectMode ? "merge" : "move_threads",
          ids,
          target: picker.value,
        };
      };
    },
    projectMode ? "합치기" : "옮기기",
  );
  if (answer) await performDrop(answer);
}
async function transferDialog(row) {
  const rootPaths = row.project.roots;
  if (!rootPaths.length) throw new Error("연결된 작업 폴더가 없습니다.");
  const answer = await dialog(
    "프로젝트 폴더 이동",
    (body) => {
      body.append(
        node(
          "p",
          "각 폴더를 새 위치로 복사·검증한 뒤 대화의 작업 경로를 갱신합니다.",
        ),
      );
      const fields = rootPaths.map((root) =>
        pathField("새 위치 · " + root.path),
      );
      for (const f of fields) body.append(f.box);
      const clean = check("검증이 끝나면 원본 폴더 정리", true);
      body.append(clean.box);
      return () => ({
        id: row.id,
        destinations: fields.map((f) => f.input.value),
        clean: clean.input.checked,
      });
    },
    "이동 시작",
  );
  if (answer) await runJob("transfer", answer);
}
async function rootsDialog() {
  const g =
    currentGroup() ??
    (state.mode === "projects" && selectedRows().length === 1
      ? group(selectedRows()[0].id)
      : null);
  if (!g) throw new Error("프로젝트 하나를 선택하세요.");
  const answer = await dialog("연결 폴더 관리", (body) => {
    body.append(
      node(
        "p",
        "연결 폴더를 바꿉니다. 파일이나 대화의 작업 경로를 옮기려면 프로젝트 폴더 이동을 사용하세요.",
      ),
    );
    const container = node("div");
    const fields = [];
    const add = (value) => {
      const f = pathField("연결 폴더", value);
      fields.push(f);
      container.append(f.box);
    };
    for (const root of g.project.roots) add(root.path);
    body.append(
      container,
      button("폴더 추가", () => add("")),
    );
    return () => ({
      id: g.project.id,
      roots: fields.map((f) => f.input.value.trim()).filter(Boolean),
    });
  });
  if (answer) await runJob("set_roots", answer);
}
async function importDialog() {
  let bundles = [];
  const answer = await dialog(
    "백업 가져오기",
    (body) => {
      body.append(
        node("p", "백업 폴더 또는 백업들을 모아 둔 상위 폴더를 선택하세요."),
      );
      const source = pathField("백업 위치");
      const destination = pathField("작업 파일을 가져올 상위 폴더");
      const choices = node("div");
      const scanStatus = node("p");
      const scan = button("백업 찾기", async () => {
        scan.disabled = true;
        scanStatus.textContent = "백업을 찾고 있습니다.";
        try {
          const id = await invoke("start_job", {
            action: "discover_backups",
            args: { path: source.input.value },
          });
          bundles = await pollJob(
            id,
            false,
            (p) =>
              (scanStatus.textContent = p.phase + " · " + duration(p.elapsed)),
          );
          choices.replaceChildren();
          for (const b of bundles) {
            const unresolved = (b.manifest.dependencies ?? []).length > 0;
            const c = check(b.manifest.project.name, !unresolved);
            c.input.disabled = unresolved;
            if (unresolved)
              c.box.append(
                node(
                  "p",
                  "외부 연결·첨부가 있어 자동 가져오기를 지원하지 않습니다. 원본에서 연결을 먼저 확인하세요.",
                  "warning",
                ),
              );
            c.box.className = "backup-choice";
            b.checkbox = c.input;
            c.box.append(
              node(
                "small",
                `${b.manifest.conversations.length}개 대화 · ${b.manifest.roots.length}개 폴더\n${b.path}`,
              ),
            );
            b.targetInputs = b.manifest.roots.map((r, index) => {
              const input = field("가져올 폴더 " + (index + 1), "");
              input.input.dataset.rootId = r.id;
              input.input.addEventListener(
                "input",
                () => (input.input.dataset.custom = "true"),
              );
              c.box.append(input.box);
              return input.input;
            });
            choices.append(c.box);
          }
          updateDestinations();
          scanStatus.textContent = bundles.length
            ? bundles.length + "개 백업을 찾았습니다."
            : "백업을 찾지 못했습니다.";
        } finally {
          scan.disabled = false;
        }
      });
      function updateDestinations() {
        for (const b of bundles) {
          for (const [index, input] of (b.targetInputs ?? []).entries()) {
            if (input.dataset.custom) continue;
            const root = b.manifest.roots[index];
            const name = safeFolderName(b.manifest.project.name);
            const basename =
              (root.original_path ?? root.id)
                .replace(/\\/g, "/")
                .split("/")
                .filter(Boolean)
                .pop() ?? root.id;
            input.value = joinPath(
              destination.input.value,
              name +
                (b.manifest.roots.length > 1
                  ? "/" + safeFolderName(basename)
                  : ""),
            );
          }
        }
      }
      destination.input.addEventListener("input", updateDestinations);
      destination.input.addEventListener("change", updateDestinations);
      const targetBox = node("label", undefined, "field");
      targetBox.append(node("span", "가져올 프로젝트"));
      const target = node("select");
      target.setAttribute("aria-label", "가져올 프로젝트");
      const fresh = node("option", "백업별로 새 프로젝트 만들기");
      fresh.value = "";
      target.append(fresh);
      for (const g of state.catalog.projects) {
        const o = node("option", g.project.name + "에 합치기");
        o.value = g.project.id;
        target.append(o);
      }
      targetBox.append(target);
      const skip = check("같은 ID의 대화가 이미 있으면 기존 기록 유지", true);
      body.append(
        source.box,
        scan,
        scanStatus,
        choices,
        destination.box,
        targetBox,
        skip.box,
      );
      return () => {
        const selected = bundles.filter((b) => b.checkbox.checked);
        if (!selected.length) throw new Error("백업을 찾아 선택하세요.");
        if (!destination.input.value)
          throw new Error("작업 파일을 가져올 상위 폴더를 선택하세요.");
        return {
          bundles: selected.map((b) => ({
            path: b.path,
            manifest: b.manifest,
            destinations: b.targetInputs.map((input) => input.value.trim()),
          })),
          destination: destination.input.value,
          target: target.value || null,
          skipExisting: skip.input.checked,
        };
      };
    },
    "가져오기 시작",
  );
  if (!answer) return;
  for (const b of answer.bundles) {
    const destinations = b.destinations;
    await runJob("import", {
      bundle: b.path,
      destinations,
      target: answer.target,
      skipExisting: answer.skipExisting,
    });
  }
}
async function loadHistory() {
  state.history = await invoke("history");
  render();
}
async function undo(id = null) {
  if (!id) {
    const history = await invoke("history");
    const supported = [
      "rust-files",
      "rust-project-edit",
      "rust-thread-names",
      "rust-assign",
      "rust-merge",
      "rust-delete",
      "rust-transfer",
      "file-edit",
      "rename-thread",
      "rename-threads",
      "management-delete",
      "connections",
    ];
    id = history.find(
      (r) =>
        r.payload.home === state.home &&
        r.state === "completed" &&
        !r.recovered &&
        !r.payload.permanent &&
        supported.includes(r.payload.kind),
    )?.id;
  }
  if (!id) throw new Error("되돌릴 작업이 없습니다.");
  if (
    await confirm(
      "작업 되돌리기",
      "선택한 작업 이전의 이름·연결·파일 상태를 복구합니다. 이후 변경된 항목은 덮어쓰지 않습니다.",
    )
  )
    await runJob("undo", { id });
}
async function resumeImport(record) {
  const p = record.payload;
  if (p.home !== state.home)
    throw new Error("작업을 시작한 저장소를 선택하세요.");
  await runJob("import", {
    bundle: p.bundle,
    destinations: p.destinations,
    target: p.target || null,
    skipExisting: true,
    resume: record.id,
  });
}
async function copy(cut = false) {
  const selected = selectedRows();
  if (!selected.length) return;
  if (state.mode === "history") return;
  state.clipboard = {
    home: state.home,
    kind: kind(),
    ids: selected.map((r) => r.id),
    cut,
  };
  if (!cut && state.mode !== "files") {
    const text = selected.map((r) => r.name).join("\n");
    try {
      await navigator.clipboard.writeText(text);
    } catch {}
    state.clipboard = null;
  }
  setStatus(
    cut ? "잘라내기 준비 · 대상에서 Ctrl+V · Esc 취소" : "복사했습니다.",
  );
  render();
}
async function paste() {
  const clip = state.clipboard;
  if (!clip) return;
  if (clip.home !== state.home)
    throw new Error("원래 저장소에서 다시 선택하세요.");
  if (clip.kind === "files") {
    const destination =
      state.mode === "files"
        ? state.directory
        : (currentGroup()?.project.roots[0]?.path ??
          selectedRows()[0]?.project?.roots[0]?.path);
    if (!destination)
      throw new Error("대상 프로젝트의 파일 폴더를 열어 주세요.");
    await performDrop({
      action: "files",
      kind: clip.cut ? "move" : "copy",
      sources: clip.ids,
      destination,
    });
  } else {
    let target = state.project;
    if (state.mode === "projects" && selectedRows().length === 1)
      target = selectedRows()[0].id;
    if (!target || clip.ids.includes(target))
      throw new Error("옮길 대상 프로젝트를 선택하세요.");
    await performDrop({
      action: clip.kind === "projects" ? "merge" : "move_threads",
      ids: clip.ids,
      target,
    });
  }
}
function context(x, y) {
  const menu = el("context");
  menu.replaceChildren();
  const item = (label, key, callback) => {
    const b = button(label, () => {
      menu.hidden = true;
      return callback();
    });
    if (key) b.append(node("kbd", key));
    menu.append(b);
  };
  item("열기", "Enter", () => {
    if (selectedRows()[0]) openRow(selectedRows()[0]);
  });
  if (state.mode !== "history") {
    item("이름 변경", "F2", rename);
    item("잘라내기", "Ctrl+X", () => copy(true));
    item("복사", "Ctrl+C", () => copy());
    item("붙여넣기", "Ctrl+V", paste);
    item("이동·합치기", "", moveDialog);
    if (state.mode === "projects") item("백업하기", "", backup);
    item("삭제", "Delete", () => deleteItems());
    if (state.mode !== "files")
      item("완전 삭제", "Shift+Delete", () => deleteItems(true));
  }
  item("되돌리기", "Ctrl+Z", () => undo());
  if (state.mode === "projects" || state.project)
    item("연결 폴더 관리", "", rootsDialog);
  menu.hidden = false;
  menu.style.left = Math.max(5, Math.min(x, window.innerWidth - 210)) + "px";
  menu.style.top =
    Math.max(5, Math.min(y, window.innerHeight - menu.offsetHeight - 8)) + "px";
}
async function settingsDialog() {
  const boot = await invoke("bootstrap");
  const answer = await dialog(
    "설정·저장소",
    (body) => {
      body.append(
        node(
          "p",
          "Codex 저장소를 바꾸거나 이전 위치에 남은 저장소를 연결할 수 있습니다.",
        ),
      );
      const picker = node("select");
      picker.setAttribute("aria-label", "연결된 Codex 저장소");
      picker.style.width = "100%";
      const fill = (stores) => {
        picker.replaceChildren();
        for (const s of stores) {
          const o = node(
            "option",
            (s.available ? "" : "연결 확인 필요 · ") + s.home,
          );
          o.value = s.home;
          o.selected = s.home === state.home;
          picker.append(o);
        }
      };
      fill(boot.stores);
      const path = pathField("Codex 저장소 경로", state.home);
      picker.addEventListener(
        "change",
        () => (path.input.value = picker.value),
      );
      const automatic = check(
        "시작할 때 업데이트 확인·미리 다운로드",
        boot.settings.startup_update_check !== false,
      );
      const binary = field(
        "Codex 실행 파일 경로 (비우면 자동 검색)",
        boot.settings.codex_binary ?? "",
      );
      const legal = node("details");
      legal.append(node("summary", "포함 구성요소 이용약관·개인정보 안내"));
      legal.append(
        node(
          "p",
          "Microsoft Defender SmartScreen이 기본 활성화되어 있으며 최종 사용자 정보를 Microsoft에 전송합니다. 개인정보 안내와 상세 약관은 아래 문서에서 확인할 수 있습니다.",
        ),
      );
      const terms = node("iframe");
      terms.src = "licenses/webview2-eula.html";
      terms.title = "포함 구성요소 이용약관";
      terms.style.width = "100%";
      terms.style.height = "250px";
      legal.append(terms);
      body.append(automatic.box, binary.box, legal);
      const scanRoot = pathField("저장소를 찾을 상위 폴더");
      const status = node("p");
      body.append(
        picker,
        path.box,
        scanRoot.box,
        button("저장소 자동 검색", async () => {
          const id = await invoke("start_job", {
            action: "scan",
            args: { roots: scanRoot.input.value ? [scanRoot.input.value] : [] },
          });
          const result = await pollJob(
            id,
            false,
            (p) => (status.textContent = p.phase + " · " + duration(p.elapsed)),
          );
          fill(result.stores);
          status.textContent = "연결된 저장소 " + result.stores.length + "개";
        }),
        status,
        node(
          "div",
          "저장소 전환은 파일을 옮기지 않습니다. 다른 컴퓨터로 옮길 때는 백업·가져오기를 사용하세요.",
          "summary-box",
        ),
      );
      return () => ({
        home: path.input.value,
        startup_update_check: automatic.input.checked,
        codex_binary: binary.input.value.trim(),
      });
    },
    "연결",
  );
  if (answer) {
    const preference = await invoke("start_job", {
      action: "preferences",
      args: answer,
    });
    await pollJob(preference, false);
    await invoke("select_home", { home: answer.home });
    state.home = answer.home;
    state.mode = "projects";
    state.project = null;
    state.clipboard = null;
    await refresh();
  }
}
el("refresh").addEventListener("click", () => refresh());
el("search").addEventListener("input", render);
el("new-project").addEventListener("click", () =>
  newProject().catch(showError),
);
el("nav-projects").addEventListener("click", () => {
  state.mode = "projects";
  state.project = null;
  state.selected.clear();
  el("search").value = "";
  renderNav();
  render();
});
el("back").addEventListener("click", () => el("nav-projects").click());
el("nav-unassigned").addEventListener("click", () => {
  state.mode = "unassigned";
  state.project = null;
  state.selected.clear();
  el("search").value = "";
  renderNav();
  render();
});
el("nav-history").addEventListener("click", () => {
  state.mode = "history";
  state.project = null;
  state.selected.clear();
  el("search").value = "";
  renderNav();
  loadHistory().catch(showError);
});
el("tab-threads").addEventListener("click", () =>
  activateProject(state.project).catch(showError),
);
el("tab-files").addEventListener("click", () =>
  activateProject(state.project, "files").catch(showError),
);
el("root-picker").addEventListener("change", () =>
  navigate(el("root-picker").value),
);
el("go-path").addEventListener("click", () => navigate(el("file-path").value));
el("file-path").addEventListener("keydown", (event) => {
  if (event.key === "Enter") navigate(el("file-path").value);
});
el("up").addEventListener("click", () => {
  const parent = state.directory
    .replace(/[\\/]+$/, "")
    .replace(/[\\/][^\\/]+$/, "");
  navigate(parent);
});
for (const [id, callback] of [
  ["rename", rename],
  ["delete", () => deleteItems()],
  ["move", moveDialog],
  ["backup", backup],
  ["import", importDialog],
  ["import-side", importDialog],
  ["settings", settingsDialog],
])
  el(id).addEventListener("click", () => callback().catch(showError));
el("updates").addEventListener("click", () => updateDialog().catch(showError));
el("mkdir").addEventListener("click", () => mkdir().catch(showError));
el("more").addEventListener("click", () => {
  const rect = el("more").getBoundingClientRect();
  context(rect.right - 190, rect.bottom + 5);
});
el("cancel").addEventListener("click", () => {
  if (state.job) {
    invoke("cancel_job", { id: state.job }).catch(showError);
    el("cancel").disabled = true;
    el("cancel").textContent = "취소 중";
  }
});
document.addEventListener("click", (event) => {
  if (!el("context").contains(event.target) && event.target !== el("more"))
    el("context").hidden = true;
});
document.addEventListener("keydown", (event) => {
  if (event.isComposing) return;
  if (el("dialog").open) return;
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
    event.preventDefault();
    el("search").focus();
    el("search").select();
    return;
  }
  const typing =
    event.target instanceof HTMLInputElement ||
    event.target instanceof HTMLTextAreaElement ||
    event.target instanceof HTMLSelectElement ||
    event.target.isContentEditable;
  if (typing) return;
  const ctrl = event.ctrlKey || event.metaKey;
  let action = null;
  if (ctrl) {
    switch (event.key.toLowerCase()) {
      case "a":
        action = () => {
          state.selected = new Set(rows().map((r) => r.id));
          render();
        };
        break;
      case "f":
        action = () => {
          el("search").focus();
          el("search").select();
        };
        break;
      case "c":
        action = () => copy();
        break;
      case "x":
        action = () => copy(true);
        break;
      case "v":
        action = paste;
        break;
      case "z":
        action = () => undo();
        break;
    }
  } else if (event.key === "F2") action = rename;
  else if (event.key === "Delete") action = () => deleteItems(event.shiftKey);
  else if (event.key === "F5") action = () => refresh();
  else if (event.key === "Enter")
    action = () => {
      if (selectedRows()[0]) openRow(selectedRows()[0]);
    };
  else if (event.key === "Escape")
    action = () => {
      state.clipboard = null;
      state.selected.clear();
      el("context").hidden = true;
      render();
    };
  else if (["ArrowDown", "ArrowUp"].includes(event.key)) {
    action = () => {
      const list = rows();
      const current = list.findIndex((r) => r.id === state.anchor);
      const index = Math.max(
        0,
        Math.min(
          list.length - 1,
          current + (event.key === "ArrowDown" ? 1 : -1),
        ),
      );
      const row = list[index];
      if (row) {
        state.selected = selectRange(
          list.map((r) => r.id),
          state.selected,
          row.id,
          state.anchor,
          event,
        );
        if (!event.shiftKey) state.anchor = row.id;
        render();
        el("rows").children[index]?.scrollIntoView({ block: "nearest" });
      }
    };
  }
  if (action) {
    event.preventDefault();
    if (!state.busy || event.key === "Escape")
      Promise.resolve(action()).catch(showError);
  }
});
async function start() {
  try {
    if (!window.__TAURI__?.core) throw new Error("데스크톱 앱에서 실행하세요.");
    const boot = await invoke("bootstrap");
    state.home = boot.home;
    state.sizes = boot.sizes ?? {};
    el("version").textContent = boot.version;
    el("notice").hidden = !boot.testMode;
    if (boot.testMode)
      el("notice").textContent = "격리 저장소를 사용하는 개발 검증 화면입니다.";
    await refresh();
    await invoke("ready");
    window.__TAURI__.event.listen("close-blocked", (event) =>
      showError(event.payload),
    );
    if (!boot.testMode && boot.settings.webview2_terms !== "2026-11-02")
      await runtimeTerms(true);
    if (
      !boot.noUpdate &&
      !boot.testMode &&
      boot.settings.startup_update_check !== false
    )
      checkUpdates(false).catch(() => {});
  } catch (error) {
    setStatus(String(error));
    el("notice").hidden = false;
    el("notice").textContent = String(error);
  }
}
start();
async function mkdir() {
  if (state.mode !== "files" || !state.directory) return;
  const answer = await dialog(
    "새 폴더 만들기",
    (body) => {
      const name = field("폴더 이름", "새 폴더");
      body.append(name.box);
      return () => name.input.value.trim();
    },
    "만들기",
  );
  if (answer)
    await runJob("mkdir", { destination: state.directory, name: answer });
}

let updateRelease = null;
let updatePreparation = null;
let updateReady = false;
let updatePreparationId = null;
async function checkUpdates(manual = true) {
  const button = el("updates");
  button.disabled = true;
  button.textContent = "업데이트 확인 중";
  try {
    const id = await invoke("start_job", { action: "update_check", args: {} });
    updateRelease = await pollJob(id, false);
    if (updateRelease.available) {
      button.textContent = "새 버전 · " + updateRelease.version;
      updatePreparation = (async () => {
        const id = await invoke("start_job", {
          action: "update_prepare",
          args: {},
        });
        updatePreparationId = id;
        await pollJob(id, false, (p) => {
          button.textContent =
            p.phase +
            (p.total ? " " + Math.floor((p.done / p.total) * 100) + "%" : "");
        });
        updateReady = true;
        button.textContent = "업데이트 설치 · " + updateRelease.version;
      })();
      updatePreparation.catch(() => {
        button.textContent = "업데이트 다운로드 재시도";
      });
    } else {
      button.textContent = "최신 버전 · " + el("version").textContent;
      if (manual)
        await dialog(
          "업데이트",
          (body) => {
            body.append(node("p", "현재 최신 버전입니다."));
            return () => true;
          },
          "닫기",
        );
    }
    return updateRelease;
  } finally {
    button.disabled = false;
  }
}
async function updateDialog() {
  if (!updateRelease?.available) {
    await checkUpdates(true);
    if (!updateRelease?.available) return;
  }
  const yes = await dialog(
    "Codex Manager " + updateRelease.version,
    (body) => {
      if (updatePreparationId && !updateReady)
        body.append(
          button("다운로드 중지", () =>
            invoke("cancel_job", { id: updatePreparationId }),
          ),
        );
      body.append(
        node(
          "p",
          updateReady
            ? "새 버전이 준비됐습니다. 업데이트하면 프로그램이 다시 시작됩니다."
            : "새 버전을 다운로드한 뒤 프로그램을 다시 시작합니다.",
        ),
        node("p", updateRelease.notes ?? ""),
      );
      return () => true;
    },
    updateReady ? "지금 업데이트" : "다운로드·업데이트",
  );
  if (!yes) return;
  if (state.busy)
    throw new Error("진행 중인 파일 관리 작업이 끝난 뒤 업데이트하세요.");
  if (!updateReady) {
    try {
      if (updatePreparation) await updatePreparation;
      else await runJob("update_prepare", {});
    } catch {
      await runJob("update_prepare", {});
    }
    updateReady = true;
  }
  setStatus("업데이트 설치 준비 중 · 완료 후 자동으로 다시 시작합니다.");
  await invoke("apply_update");
}

async function runtimeTerms(required) {
  const yes = await dialog(
    "포함 구성요소 이용약관",
    (body) => {
      body.append(
        node(
          "p",
          "Codex Manager에는 Microsoft Edge WebView2 Runtime이 포함됩니다. Microsoft Defender SmartScreen은 기본으로 활성화되어 있으며, Microsoft 개인정보처리방침 및 Edge 개인정보 백서에 설명된 최종 사용자 정보를 Microsoft에 전송합니다.",
        ),
      );
      for (const [text, url] of [
        ["Microsoft 개인정보처리방침", "https://aka.ms/privacy"],
        [
          "Edge SmartScreen 개인정보 안내",
          "https://learn.microsoft.com/en-us/microsoft-edge/privacy-whitepaper#smartscreen",
        ],
      ]) {
        const link = node("a", text);
        link.href = url;
        link.target = "_blank";
        link.rel = "noreferrer";
        body.append(link, node("br"));
      }
      const frame = node("iframe");
      frame.src = "licenses/webview2-eula.html";
      frame.title = "Microsoft Edge WebView2 Runtime 이용약관";
      frame.style.width = "100%";
      frame.style.height = "250px";
      body.append(frame);
      const accept = check(
        "포함된 WebView2 Runtime에 적용되는 Microsoft 이용약관에 동의합니다.",
        false,
      );
      body.append(accept.box);
      return () => {
        if (!accept.input.checked) throw new Error("동의 여부를 선택하세요.");
        return true;
      };
    },
    "동의하고 계속",
  );
  if (yes) {
    const id = await invoke("start_job", { action: "accept_terms", args: {} });
    await pollJob(id, false);
  } else if (required) await invoke("quit");
}
