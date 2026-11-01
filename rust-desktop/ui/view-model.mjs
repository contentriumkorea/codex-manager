export function visibleProjects(catalog, query, direction) {
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
        ...(threads ?? []).map((thread) => thread.name ?? ""),
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
