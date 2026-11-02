export const normalizeHome = (home) =>
  String(home ?? "")
    .replaceAll("/", "\\")
    .replace(/[\\]+$/, "")
    .toLocaleLowerCase("en-US");
export const sameHome = (a, b) => normalizeHome(a) === normalizeHome(b);
export const keyboardTarget = (
  active,
  { editing = false, dialog = false } = {},
) => (editing || dialog ? null : active);
const clone = (value) => JSON.parse(JSON.stringify(value));
export class PaneModel {
  constructor() {
    this.panes = new Map();
    this.active = "left";
    this.clipboard = null;
    this.drag = null;
  }
  update(pane, snapshot) {
    if (!["left", "right"].includes(pane)) throw new Error("Unknown pane");
    const previous = this.panes.get(pane);
    const data = clone(snapshot);
    const changed = JSON.stringify(previous?.data) !== JSON.stringify(data);
    this.panes.set(pane, {
      data,
      generation: (previous?.generation ?? 0) + (changed ? 1 : 0),
    });
  }
  snapshot(pane) {
    const entry = this.panes.get(pane);
    return entry ? clone(entry.data) : null;
  }
  generation(pane) {
    return this.panes.get(pane)?.generation;
  }
  current(pane, generation) {
    return generation !== undefined && this.generation(pane) === generation;
  }
  capture(pane) {
    const data = this.snapshot(pane);
    if (!data || !data.ids.length || data.mode === "history") return null;
    return {
      ...data,
      pane,
      generation: this.generation(pane),
      kind:
        data.mode === "projects"
          ? "projects"
          : data.mode === "files"
            ? "files"
            : "threads",
    };
  }
  validSource(source) {
    return !!source && this.current(source.pane, source.generation);
  }
  copy(pane, cut = false) {
    const source = this.capture(pane);
    this.clipboard = source ? { ...source, copy: !cut } : null;
    return this.clipboard;
  }
  beginDrag(pane) {
    const source = this.capture(pane);
    this.drag = source ? { token: crypto.randomUUID(), source } : null;
    return this.drag?.token;
  }
  dragSource(token) {
    return token && token === this.drag?.token ? clone(this.drag.source) : null;
  }
  ticket(source, target) {
    return {
      source: clone(source),
      target,
      generation: this.generation(target),
    };
  }
  valid(ticket) {
    return (
      this.validSource(ticket.source) &&
      this.current(ticket.target, ticket.generation)
    );
  }
}
