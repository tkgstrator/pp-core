import init, { script_flow, WebSession } from "./pkg/pp_core.js";
import { AudioPlayer } from "./audio.js";
import { FlowMap } from "./flow.js";
import { StateInspector } from "./state.js";

const byId = (id) => document.getElementById(id);
const stage = byId("stage");
const background = byId("background");
const choices = byId("choices");
const line = byId("line");
const speaker = byId("speaker");
const next = byId("next");
const start = byId("start");
const error = byId("error");
const audio = new AudioPlayer();
const flow = new FlowMap(byId("flow"), byId("flow-scroll"), byId("flow-empty"));
const inspector = new StateInspector(byId);
const assets = new Map();
let session;
let timer;
let effect;
let demoMode = true;
let lastView;
let playerName;

function showError(problem) {
  error.textContent = problem instanceof Error ? problem.message : String(problem);
  error.hidden = false;
}

function clearError() {
  error.hidden = true;
}

async function fetchText(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`ファイルを読み込めませんでした (${response.status})`);
  return response.text();
}

async function loadDemo() {
  const [document, scenario] = await Promise.all([
    fetchText("../docs/pp-core/scripts/format.md"),
    fetchText("./scenario.yaml"),
  ]);
  const script = document.match(/^\x60\x60\x60yaml\r?\n([\s\S]*?)^\x60\x60\x60/m)?.[1];
  if (!script) throw new Error("仕様書のサンプルが見つかりません。");
  return { script, scenario };
}

const ready = Promise.all([init(), loadDemo()]);
function showFlow(source) {
  flow.load(source, (yaml) => JSON.parse(script_flow(yaml)));
}

ready.then(([, demo]) => {
  showFlow(demo.script);
  start.disabled = false;
}).catch(showError);

byId("script").addEventListener("change", async (event) => {
  const input = event.currentTarget;
  const file = input.files[0];
  try {
    const [, demo] = await ready;
    const source = file ? await file.text() : demo.script;
    if (file === input.files[0] && (!session || lastView?.pause.type === "ended")) {
      clearError();
      showFlow(source);
    }
  } catch (problem) {
    showError(problem);
  }
});

function assetUrl(id, type) {
  const asset = assets.get(id);
  return asset?.type.startsWith(type + "/") ? asset.url : undefined;
}

function renderPortrait(side, id) {
  const element = byId(side);
  if (element.dataset.asset === (id ?? "")) return;
  element.dataset.asset = id ?? "";
  element.hidden = !id;
  element.replaceChildren();
  if (!id) return;

  const url = assetUrl(id, "image");
  if (url) {
    const image = document.createElement("img");
    image.alt = "";
    image.src = url;
    element.append(image);
  } else {
    const label = document.createElement("span");
    label.textContent = id === "protagonist" ? playerName
      : (demoMode && id === "hazuki" ? "葉月" : id);
    element.append(label);
  }
}

function renderScene(state) {
  stage.dataset.background = state.background ?? "";
  const bgUrl = assetUrl(state.background, "image");
  background.hidden = !bgUrl;
  if (bgUrl && background.src !== bgUrl) background.src = bgUrl;
  if (!bgUrl) background.removeAttribute("src");
  renderPortrait("left", state.left);
  renderPortrait("right", state.right);
}

function playEvents(events) {
  for (const event of events) {
    switch (event.type) {
      case "start_bgm":
        audio.startBgm(assetUrl(event.asset, "audio"), demoMode && event.asset === "after_school");
        break;
      case "stop_bgm":
        audio.stopBgm();
        break;
      case "play_se":
        audio.playSe(assetUrl(event.asset, "audio"), demoMode && event.asset === "thunder");
        break;
      case "play_effect":
        if (event.effect === "shake" && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
          effect?.cancel();
          effect = stage.animate([
            { transform: "translateX(0)" }, { transform: "translateX(-7px)" },
            { transform: "translateX(6px)" }, { transform: "translateX(-3px)" },
            { transform: "translateX(0)" },
          ], { duration: 360 });
        }
        break;
    }
  }
}

function wait(ms) {
  const until = performance.now() + ms;
  const tick = () => {
    const remaining = until - performance.now();
    if (remaining <= 0) step("resume");
    else timer = setTimeout(tick, Math.min(remaining, 2_147_483_647));
  };
  tick();
}

function render(state) {
  lastView = state;
  byId("scene-id").textContent = `SCENE ${state.scene ?? "—"}`;
  renderScene(state);
  const pause = state.pause;
  speaker.hidden = pause.type !== "text" || !pause.speaker;
  speaker.textContent = speaker.hidden ? "" : pause.speaker;
  next.hidden = pause.type !== "text";
  choices.replaceChildren();
  switch (pause.type) {
    case "text":
      line.textContent = pause.text;
      break;
    case "choice":
      line.textContent = "どうする？";
      for (const [index, text] of pause.options.entries()) {
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = text;
        button.addEventListener("click", () => step("choose", index));
        choices.append(button);
      }
      choices.firstElementChild?.focus({ preventScroll: true });
      break;
    case "wait":
      line.textContent = "…";
      wait(pause.ms);
      break;
    case "ended":
      line.textContent = "シーン終了";
      start.focus({ preventScroll: true });
      break;
  }
}

function update(raw, choiceIndex) {
  const { state, events, branches, changes } = JSON.parse(raw);
  inspector.update(state, changes);
  flow.record(state, branches, choiceIndex);
  playEvents(events);
  render(state);
}

function step(method, ...args) {
  if (!session) return;
  try {
    update(session[method](...args), method === "choose" ? args[0] : undefined);
  } catch (problem) {
    audio.reset();
    lastView = undefined;
    speaker.hidden = true;
    next.hidden = true;
    choices.replaceChildren();
    showError(problem);
  }
}

byId("setup").addEventListener("submit", async (event) => {
  event.preventDefault();
  clearError();
  const player = byId("player").value.trim();
  if (!player) {
    showError("プレイヤー名を入力してください。");
    return;
  }
  start.disabled = true;
  try {
    audio.unlock();
    const [, demo] = await ready;
    const scriptFile = byId("script").files[0];
    const scenarioFile = byId("scenario").files[0];
    if (scriptFile && !scenarioFile) {
      throw new Error("スクリプトと一緒に scenario.yaml を選択してください。");
    }
    const [script, scenario] = await Promise.all([
      scriptFile ? scriptFile.text() : demo.script,
      scenarioFile ? scenarioFile.text() : demo.scenario,
    ]);
    const replacement = new WebSession(script, scenario, player);
    showFlow(script);
    clearTimeout(timer);
    effect?.cancel();
    audio.reset();
    session?.free();
    session = replacement;
    playerName = player;
    demoMode = !scriptFile;
    delete byId("left").dataset.asset;
    delete byId("right").dataset.asset;
  } catch (problem) {
    showError(problem);
    return;
  } finally {
    start.disabled = false;
  }
  flow.reset();
  inspector.reset();
  step("start");
});

next.addEventListener("click", () => {
  step("advance");
  if (!next.hidden) next.focus({ preventScroll: true });
});

byId("sound").addEventListener("click", (event) => {
  const button = event.currentTarget;
  const enabled = button.getAttribute("aria-pressed") !== "true";
  audio.setEnabled(enabled);
  button.setAttribute("aria-pressed", String(enabled));
  button.textContent = enabled ? "音声オン" : "音声オフ";
});

byId("assets").addEventListener("change", (event) => {
  audio.reset();
  for (const asset of assets.values()) URL.revokeObjectURL(asset.url);
  assets.clear();
  for (const file of event.currentTarget.files) {
    const id = file.name.replace(/\.[^.]+$/, "");
    const previous = assets.get(id);
    if (previous) URL.revokeObjectURL(previous.url);
    assets.set(id, { type: file.type, url: URL.createObjectURL(file) });
  }
  if (lastView) {
    delete byId("left").dataset.asset;
    delete byId("right").dataset.asset;
    renderScene(lastView);
    if (lastView.bgm) playEvents([{ type: "start_bgm", asset: lastView.bgm }]);
  }
});
