export function visibleThreads(threads = [], filter = "normal") {
  const internal = filter.startsWith("all");
  const active = filter.endsWith("active");
  return threads.filter((t) => (internal || !t.internal) && (!active || !t.archived));
}
export function membershipLabel(thread) {
  return ({explicit: "실제 연결", folder: "폴더 기준 · 표시만", parent: "원래 대화 기준 · 표시만",
    projectless: "프로젝트 없음으로 설정", ambiguous: "여러 프로젝트 · 확인 필요",
    missing_project: "원래 프로젝트 없음", other_host: "다른 기기 연결",
    unresolved_assignment: "기존 연결 확인 필요", internal_parent: "원래 대화 연결 확인 필요",
    outside: "연결 근거 없음"})[thread.membershipReason] || "연결 확인 필요";
}
export function visibleProjects(catalog, query, direction, filter = "normal") {
  const term = query.trim().toLocaleLowerCase("ko-KR");
  const collator = new Intl.Collator("ko-KR", {
    numeric: true,
    sensitivity: "base",
  });
  return (catalog.projects ?? [])
    .filter(({ project, threads }) => {
      const values = [
        project.name ?? "",
        ...(project.roots ?? []).map((root) => root.path ?? ""),
        ...visibleThreads(threads, filter).map((thread) => thread.name ?? ""),
      ];
      return values.some((value) =>
        value.toLocaleLowerCase("ko-KR").includes(term),
      );
    })
    .slice()
    .sort((a, b) => {
      const order = collator.compare(
        a.project.name ?? "",
        b.project.name ?? "",
      );
      return direction === "desc" ? -order : order;
    });
}
