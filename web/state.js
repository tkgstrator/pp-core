function display(value) {
  return value == null ? "—" : String(value);
}

export class StateInspector {
  #previous;

  constructor(byId) {
    this.scroll = byId("state-scroll");
    this.empty = byId("state-empty");
    this.content = byId("state-content");
    this.history = byId("state-history");
    this.historyEmpty = byId("state-history-empty");
    this.groups = {
      stat: byId("state-stats"),
      flag: byId("state-flags"),
      session: byId("state-session"),
    };
  }

  reset() {
    this.#previous = undefined;
    this.history.replaceChildren();
    this.historyEmpty.hidden = false;
    this.content.hidden = true;
    this.empty.hidden = false;
    this.scroll.scrollTop = 0;
  }

  update(state, changes) {
    const groups = {
      stat: state.stats,
      flag: state.flags,
      session: {
        scene: state.scene,
        position: state.location?.path,
        status: state.pause.type === "wait" ? `wait ${state.pause.ms} ms` : state.pause.type,
        background: state.background,
        left: state.left,
        right: state.right,
        bgm: state.bgm,
      },
    };
    const firstChanges = { stat: new Map(), flag: new Map() };
    for (const {kind, id, before, after} of changes) {
      if (!firstChanges[kind].has(id)) firstChanges[kind].set(id, before);
      const line = document.createElement("li");
      const label = document.createElement("span");
      const transition = document.createElement("span");
      label.textContent = `${kind}.${id}`;
      transition.textContent = `${display(before)} → ${display(after)}`;
      transition.className = "state-transition";
      line.append(label, transition);
      this.history.prepend(line);
    }
    while (this.history.children.length > 100) this.history.lastElementChild.remove();
    this.historyEmpty.hidden = this.history.children.length > 0;

    for (const [kind, values] of Object.entries(groups)) {
      this.#renderGroup(this.groups[kind], values, this.#previous?.[kind], firstChanges[kind]);
    }
    this.#previous = groups;
    this.empty.hidden = true;
    this.content.hidden = false;
  }

  #renderGroup(element, values, previous, executed) {
    element.replaceChildren();
    for (const [id, value] of Object.entries(values)) {
      const row = document.createElement("div");
      const name = document.createElement("dt");
      const cell = document.createElement("dd");
      const changed = executed?.has(id) || (previous && Object.hasOwn(previous, id) && previous[id] !== value);
      name.textContent = id;
      cell.textContent = display(value);
      if (changed) {
        const old = executed?.has(id) ? executed.get(id) : previous[id];
        const before = document.createElement("span");
        before.className = "state-before";
        before.textContent = `${display(old)} → `;
        cell.prepend(before);
        row.className = "state-changed";
      }
      row.append(name, cell);
      element.append(row);
    }
    if (!element.children.length) {
      const row = document.createElement("div");
      const name = document.createElement("dt");
      const value = document.createElement("dd");
      name.textContent = "未定義";
      value.textContent = "—";
      row.append(name, value);
      element.append(row);
    }
  }
}
