function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function conditionText(condition) {
  const comparisons = { eq: "=", ne: "!=", lt: "<", le: "<=", gt: ">", ge: ">=" };
  const op = Object.keys(comparisons).find((key) => condition[key] != null);
  return `${condition.stat ?? condition.flag} ${comparisons[op]} ${condition[op]}`;
}

export class FlowMap {
  #scenes = new Map();
  #nodes = new Map();
  #active;
  #source;

  constructor(list, scroll, empty) {
    this.list = list;
    this.scroll = scroll;
    this.empty = empty;
  }

  load(source, parse) {
    if (this.#source === source) return;
    const scenes = parse(source);
    this.#source = source;
    this.list.replaceChildren();
    this.#scenes.clear();
    this.#nodes.clear();
    this.#active = undefined;
    this.empty.hidden = scenes.length > 0;
    for (const scene of scenes) {
      const item = element("li", "flow-scene");
      const heading = element("h3", "flow-scene-title", scene.scene);
      const details = [];
      if (scene.when != null) details.push(`WHEN ${scene.when}`);
      if (scene.phase != null) details.push(`PHASE ${scene.phase}`);
      if (scene.if != null) details.push(`IF ${conditionText(scene.if)}`);
      if (scene.prob != null) details.push(`PROB ${scene.prob}%`);
      item.append(heading);
      if (details.length) item.append(element("p", "flow-conditions", details.join(" · ")));
      item.append(this.#commands(scene.scene, "commands", scene.commands));
      this.#scenes.set(scene.scene, item);
      this.list.append(item);
    }
    this.scroll.scrollTop = 0;
  }

  reset() {
    this.#active = undefined;
    for (const node of this.list.querySelectorAll(".visited, .selected, .current-scene, [aria-current]")) {
      node.classList.remove("visited", "selected", "current-scene");
      node.removeAttribute("aria-current");
    }
    this.scroll.scrollTop = 0;
  }

  record(state, branches, choiceIndex) {
    if (choiceIndex !== undefined) this.#select(this.#active, choiceIndex);
    for (const branch of branches) {
      const entry = this.#get(branch.at);
      entry?.node.classList.add("visited");
      if (branch.type === "if") this.#select(entry, branch.matched ? 0 : 1);
      if (branch.type === "switch") this.#select(entry, branch.selected ?? branch.cases.length);
    }

    this.list.querySelector("[aria-current]")?.removeAttribute("aria-current");
    this.#active = this.#get(state.location);
    const node = this.#active?.node ?? this.#scenes.get(state.scene);
    for (const scene of this.#scenes.values()) scene.classList.toggle("current-scene", scene === this.#scenes.get(state.scene));
    if (node) {
      this.#active?.node.classList.add("visited");
      node.setAttribute("aria-current", "step");
      this.#scrollTo(node);
    }
  }

  #get(location) {
    return location ? this.#nodes.get(JSON.stringify([location.scene, location.path])) : undefined;
  }

  #commands(scene, prefix, commands) {
    const list = element("ol", "flow-steps");
    commands.forEach((command, index) => {
      const path = `${prefix}.${index}`;
      const node = element("li", "flow-node");
      const entry = { node, options: [] };
      const label = command.type.toUpperCase();
      let text;
      let options;
      switch (command.type) {
        case "text": text = command.speaker ?? ""; break;
        case "choice":
          options = command.options.map((option, i) => [option.text, `options.${i}`, option.commands]);
          break;
        case "if":
          text = conditionText(command.condition);
          options = [["then", "then", command.then], ["else", "else", command.else]];
          break;
        case "switch":
          text = command.stat ?? command.flag;
          options = [
            ...command.cases.map(({ case: item }, i) => [`case ${item.value}`, `cases.${i}`, item.commands]),
            ["default", "default", command.default],
          ];
          break;
        case "go": {
          const link = element("button", "flow-link", `→ ${command.scene}`);
          link.type = "button";
          link.addEventListener("click", () => this.#scrollTo(this.#scenes.get(command.scene)));
          text = link;
          break;
        }
        case "wait": text = `${command.ms} ms`; break;
        case "end": break;
        default: return;
      }

      const heading = element("div", "flow-row");
      heading.append(element("span", "flow-label", label));
      if (typeof text === "string" && text) heading.append(element("span", "flow-value", text));
      else if (text instanceof HTMLElement) heading.append(text);
      node.append(heading);

      if (options) {
        const fork = element("ul", "flow-fork");
        for (const [title, suffix, children] of options) {
          const option = element("li", "flow-option");
          const name = element("span", "flow-option-name", title);
          if (children.length === 0) name.append(element("span", "flow-continue", " → 続行"));
          option.append(name);
          if (children.length) option.append(this.#commands(scene, `${path}.${suffix}`, children));
          fork.append(option);
          entry.options.push(option);
        }
        node.append(fork);
      }
      list.append(node);
      this.#nodes.set(JSON.stringify([scene, path]), entry);
    });
    if (!list.children.length) list.hidden = true;
    return list;
  }

  #select(entry, index) {
    const option = entry?.options[index];
    if (option) option.classList.add("selected");
  }

  #scrollTo(node) {
    if (!node) return;
    const row = node.firstElementChild ?? node;
    const pane = this.scroll.getBoundingClientRect();
    const { top, bottom } = row.getBoundingClientRect();
    if (top < pane.top || bottom > pane.bottom) {
      this.scroll.scrollTop += top - pane.top - 12;
    }
  }
}
