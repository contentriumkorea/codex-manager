export function visibleFolders(folders, query = "") {
  const term = query.trim().toLocaleLowerCase();
  return folders.filter((f) => f.name.toLocaleLowerCase().includes(term));
}
export function folderTransferRequest(source, destination, copy) {
  if (!source?.ids?.length) throw new Error("메인 화면에서 프로젝트 또는 파일을 선택하세요.");
  if (!destination) throw new Error("대상 폴더를 선택하세요.");
  if (!["projects", "files"].includes(source.kind)) throw new Error("대화 합치기·이동은 메인 화면에서 대상 프로젝트를 선택하세요.");
  return source.kind === "files"
    ? { action: "files_to_folder", args: { home: source.home, sources: source.ids, destination, kind: copy ? "copy" : "move", expectedRevision: source.revision } }
    : { action: "portable_transfer", args: { home: source.home, ids: source.ids, destination, copy, expectedRevision: source.revision } };
}
