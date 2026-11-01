const collator = new Intl.Collator("ko-KR", {
  numeric: true,
  sensitivity: "base",
});
export function sortedRows(rows, key, direction) {
  return rows.slice().sort((a, b) => {
    if (a.directory !== undefined && a.directory !== b.directory)
      return a.directory ? -1 : 1;
    const av = a[key],
      bv = b[key];
    const order =
      typeof av === "number" && typeof bv === "number"
        ? av - bv
        : collator.compare(String(av ?? ""), String(bv ?? ""));
    return direction === "desc" ? -order : order;
  });
}
export function selectRange(
  ids,
  current,
  clicked,
  anchor,
  { ctrlKey = false, metaKey = false, shiftKey = false } = {},
) {
  const selected = new Set(current);
  if (shiftKey && anchor !== null && ids.includes(anchor)) {
    if (!ctrlKey && !metaKey) selected.clear();
    const a = ids.indexOf(anchor),
      b = ids.indexOf(clicked);
    for (const id of ids.slice(Math.min(a, b), Math.max(a, b) + 1))
      selected.add(id);
  } else if (ctrlKey || metaKey) {
    if (selected.has(clicked)) selected.delete(clicked);
    else selected.add(clicked);
  } else {
    selected.clear();
    selected.add(clicked);
  }
  return selected;
}
export function dropPlan(payload, target, home, copy = false) {
  if (!payload || payload.home !== home || !payload.ids?.length) return null;
  if (payload.kind === "projects") {
    if (!target.projectId || payload.ids.includes(target.projectId))
      return null;
    return { action: "merge", ids: payload.ids, target: target.projectId };
  }
  if (payload.kind === "threads") {
    if (!target.projectId) return null;
    return {
      action: "move_threads",
      ids: payload.ids,
      target: target.projectId,
    };
  }
  if (payload.kind === "files" && target.directory) {
    return {
      action: "files",
      kind: copy ? "copy" : "move",
      sources: payload.ids,
      destination: target.directory,
    };
  }
  return null;
}
export function formatBytes(value) {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "0 B";
  const unit = Math.min(4, Math.floor(Math.log(value) / Math.log(1024)));
  return (
    (value / 1024 ** unit).toLocaleString("ko-KR", {
      maximumFractionDigits: unit ? 1 : 0,
    }) +
    " " +
    ["B", "KB", "MB", "GB", "TB"][unit]
  );
}
export function duration(seconds) {
  seconds = Math.max(0, Math.floor(seconds));
  if (seconds < 60) return seconds + "초";
  if (seconds < 3600)
    return Math.floor(seconds / 60) + "분 " + (seconds % 60) + "초";
  return (
    Math.floor(seconds / 3600) +
    "시간 " +
    Math.floor((seconds % 3600) / 60) +
    "분"
  );
}
export function safeFolderName(name) {
  const value = name
    .replace(/[<>:"/\\|?*\x00-\x1f]/g, "_")
    .replace(/[. ]+$/g, "")
    .trim();
  return value || "프로젝트";
}
export function joinPath(parent, name) {
  return parent.replace(/[\\/]+$/, "") + "\\" + name;
}
