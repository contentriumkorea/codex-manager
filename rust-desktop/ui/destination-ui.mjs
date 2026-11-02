import { visibleFolders } from "./destination-model.mjs";
export function destinationBrowser({ element: el, invoke, job, error, transfer, document: doc = globalThis.document }) {
  let current = "", parent = "", folders = [], generation = 0;
  function render() {
    el("destination-list").replaceChildren();
    for (const folder of visibleFolders(folders, el("destination-search").value)) {
      const button = doc.createElement("button");
      button.type = "button"; button.className = "destination-folder";
      button.textContent = "▱ " + folder.name; button.title = folder.path;
      button.addEventListener("dblclick", () => navigate(folder.path).catch(error));
      button.addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); navigate(folder.path).catch(error); } });
      button.addEventListener("click", () => { el("destination-hint").textContent = folder.name + " · 두 번 클릭하여 열기"; });
      el("destination-list").append(button);
    }
  }
  async function navigate(path) {
    if (!path) return;
    const ticket = ++generation;
    const data = await job("folder_list", { path });
    if (ticket !== generation) return;
    current = data.path; parent = data.parent; folders = data.folders;
    el("destination-address").value = current;
    el("destination-current").textContent = current;
    el("destination-current").title = current;
    el("destination-up").disabled = !parent;
    el("destination-hint").textContent = "현재 폴더에 전송 · 하위 폴더는 두 번 클릭하여 열기";
    render();
    await job("pane_preferences", { pane: "right", destination: current });
  }
  el("destination-search").addEventListener("input", render);
  el("destination-address").addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); navigate(event.target.value.trim()).catch(error); } });
  el("destination-go").addEventListener("click", () => navigate(el("destination-address").value.trim()).catch(error));
  el("destination-up").addEventListener("click", () => navigate(parent).catch(error));
  el("destination-pick").addEventListener("click", async () => { try { const path = await invoke("choose_folder", { initial: current || null }); if (path) await navigate(path); } catch (e) { error(e); } });
  el("destination-copy").addEventListener("click", () => transfer(true).catch(error));
  el("destination-move").addEventListener("click", () => transfer(false).catch(error));
  return { navigate, path: () => current, refresh: () => current ? navigate(current) : Promise.resolve() };
}
