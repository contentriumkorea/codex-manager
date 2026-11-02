export function portableUI({ dialog, job, run, invoke, node, snapshot, error, element }) {
  const time = (value) => value ? (typeof value === "number" ? new Date(value * 1000).toLocaleString() : String(value)) : "아직 없음";
  const statusFailure = (status) => status.statusError ? "현재 보관 상태 확인 실패: " + status.statusError : "";
  async function controls(project) {
    const source = snapshot();
    const roots = project.roots ?? [];
    if (!roots.length) throw new Error("먼저 프로젝트 폴더를 연결하세요.");
    let anchor = roots[0].path;
    let status = await job("portable_status", { home: source.home, project: project.id, anchor });
    if (status.registration?.anchor && roots.some((r) => r.path === status.registration.anchor) && status.registration.anchor !== anchor) {
      anchor = status.registration.anchor;
      status = await job("portable_status", { home: source.home, project: project.id, anchor });
    }
    const answer = await dialog("left", "대화 자동 보관", (body) => {
      const picker = node("select"); picker.setAttribute("aria-label", "대화를 보관할 프로젝트 폴더");
      for (const root of roots) { const option = node("option", root.path); option.value = root.path; picker.append(option); }
      picker.value = anchor;
      const enabled = node("input"); enabled.type = "checkbox"; enabled.checked = status.registration?.enabled === true;
      const label = node("label"); label.append(enabled, node("span", " 이 프로젝트의 대화 자동 보관"));
      const details = node("div", "", "summary-box"); details.setAttribute("aria-live", "polite");
      const render = () => { details.textContent = `마지막 성공: ${time(status.snapshot?.lastSuccessfulSave ?? status.registration?.lastSuccessfulSave)}\n${statusFailure(status) || (status.snapshot?.complete ? "마지막 정상 보관본 있음" : "정상 보관본 없음")}\n${status.registration?.error || status.snapshot?.lastAttempt?.error || ""}\n${status.registration?.retentionWarning || status.snapshot?.lastAttempt?.retentionWarning || ""}`; };
      const reload = async () => { status = await job("portable_status", { home: source.home, project: project.id, anchor }); render(); };
      picker.addEventListener("change", () => { anchor = picker.value; reload().catch(error); });
      const save = node("button", "지금 보관"); save.type = "button";
      save.addEventListener("click", async () => { save.disabled = true; try { await run("left", "portable_save", { home: source.home, project: project.id, anchor }); await reload(); } catch (e) { await reload(); error(e); } finally { save.disabled = false; } });
      body.append(node("p", project.name), picker, label, details, save, node("p", "Manager가 실행 중일 때 약 1분마다 변경을 확인합니다. 트레이에 있어도 동작하며 완전 종료 후에는 저장하지 않습니다. 일반 파일은 현재 폴더에 그대로 둡니다."));
      render();
      return () => ({ enabled: enabled.checked, anchor });
    }, "설정 저장");
    if (answer) await run("left", "portable_configure", { home: source.home, project: project.id, ...answer });
  }
  async function openFolder() {
    const path = await invoke("choose_folder", { initial: null });
    if (!path) return;
    const found = await run("left", "portable_discover", { path });
    const source = snapshot();
    let selected = null, target = null, plan = null, generation = 0;
    const answer = await dialog("left", "폴더 열기", (body) => {
      const projects = node("select"); projects.setAttribute("aria-label", "보관된 프로젝트");
      const empty = node("option", "보관된 프로젝트 선택"); empty.value = ""; projects.append(empty);
      for (let i = 0; i < found.projects.length; i++) { const p = found.projects[i]; const option = node("option", `${p.project?.name ?? "프로젝트"} · ${p.anchor}${p.complete ? "" : " · 보관 실패"}`); option.value = String(i); projects.append(option); }
      const merge = node("select"); merge.setAttribute("aria-label", "가져오기 방식");
      const fresh = node("option", "새 프로젝트로 열기"); fresh.value = ""; merge.append(fresh);
      for (const project of source.projects ?? []) { const option = node("option", "합치기: " + project.name); option.value = project.id; merge.append(option); }
      const details = node("pre", "프로젝트를 선택하면 대화와 폴더 연결을 검증합니다.", "summary-box transfer-mapping");
      const update = async () => {
        const ticket = ++generation; plan = null; element("dialog-ok").disabled = true;
        selected = projects.value === "" ? null : found.projects[Number(projects.value)]; target = merge.value || null;
        if (!selected) return;
        details.textContent = "보관본·폴더 연결·기존 대화 충돌 확인 중…";
        try {
          const inspected = await run("left", "portable_inspect", { anchor: selected.anchor, namespace: selected.namespace });
          const preview = await run("left", "portable_plan", { home: source.home, anchor: selected.anchor, namespace: selected.namespace, target });
          if (ticket !== generation || !element("dialog").open || details.isConnected === false) return;
          plan = { ...preview, namespace: selected.namespace, target, home: source.home };
          details.textContent = `${inspected.project.name} · 대화 ${preview.conversationCount}개\n연결 폴더:\n${preview.roots.join("\n")}\n\n충돌: ${preview.conflicts.length}\n${preview.conflicts.map((c) => c.reason).join("\n")}\n동일한 기존 대화: ${preview.skipped.length}개\n일반 파일은 선택한 폴더에서 그대로 사용합니다.`;
          element("dialog-ok").disabled = !preview.canRestore;
        } catch (e) { if (ticket === generation && details.isConnected !== false) details.textContent = String(e); }
      };
      projects.addEventListener("change", update); merge.addEventListener("change", update);
      body.append(projects, merge, details);
      if (found.diagnostics?.length) body.append(node("pre", found.diagnostics.map((d) => `${d.path}: ${d.error}`).join("\n")));
      if (found.truncated) body.append(node("p", "검색 범위가 커 일부 폴더만 확인했습니다. 더 가까운 폴더를 선택하세요."));
      if (!found.projects.length) body.append(node("p", "이 폴더에서 대화 보관본을 찾지 못했습니다."));
      element("dialog-ok").disabled = true;
      return () => { if (!plan?.canRestore) throw new Error("검증된 프로젝트를 선택하세요."); return plan; };
    }, "확인한 폴더 열기");
    if (!answer) return;
    const latest = await run("left", "portable_plan", { home: answer.home, anchor: answer.anchor, namespace: answer.namespace, target: answer.target });
    if (latest.snapshotId !== answer.snapshotId || JSON.stringify(latest.conflicts) !== JSON.stringify(answer.conflicts) || !latest.canRestore) throw new Error("보관본이나 충돌 상태가 바뀌었습니다. 폴더를 다시 열어 확인하세요.");
    return run("left", "portable_restore", { home: answer.home, anchor: answer.anchor, namespace: answer.namespace, target: answer.target });
  }
  async function summary(project) {
    const root = project.roots?.[0]?.path;
    if (!root) return "대화 보관: 연결 폴더 없음";
    const status = await job("portable_status", { home: snapshot().home, project: project.id, anchor: root });
    const failure = status.registration?.error || status.snapshot?.lastAttempt?.error;
    return `자동 보관 ${status.registration?.enabled ? "켜짐" : "꺼짐"} · 마지막 성공 ${time(status.snapshot?.lastSuccessfulSave ?? status.registration?.lastSuccessfulSave)}${status.statusError ? "\n" + statusFailure(status) : ""}${failure ? "\n보관 실패: " + failure : ""}`;
  }
  return { controls, openFolder, summary };
}
