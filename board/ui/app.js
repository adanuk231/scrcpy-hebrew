/* The board: one card per phone, none of them holding a picture.
   Everything that needs a restart says so; everything that does not is sent
   straight to the phone over adb while it keeps mirroring. */

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const rack = document.getElementById("rack");
const note = document.getElementById("note");
const tmpl = document.getElementById("pedal-template");

const DEFAULTS = {
  audio: "off",
  max_size: 0,
  max_fps: 0,
  stay_awake: true,
  screen_off: false,
  show_touches: false,
  view_only: false,
  skin: false,
  height: 760,
};

/** serial -> capability record from the python probe */
const caps = new Map();
/** serial -> chosen options, remembered between runs */
const opts = new Map();
/** serials currently mirroring */
let live = new Set();
/** the order the cards are in, which is also the order "line them up" uses */
let order = [];

// ------------------------------------------------------------ settings ---

function load() {
  try {
    const raw = JSON.parse(localStorage.getItem("board") || "{}");
    order = raw.order || [];
    for (const [serial, value] of Object.entries(raw.opts || {})) {
      opts.set(serial, { ...DEFAULTS, ...value });
    }
  } catch (_) {
    /* first run */
  }
}

function save() {
  localStorage.setItem("board", JSON.stringify({
    order,
    opts: Object.fromEntries(opts),
  }));
}

function optionsFor(serial) {
  if (!opts.has(serial)) {
    const cap = caps.get(serial) || {};
    opts.set(serial, {
      ...DEFAULTS,
      // a phone that cannot forward audio should not pretend it is muted
      audio: "off",
      keyboard: cap.keyboard || "paste",
    });
  }
  const value = opts.get(serial);
  const cap = caps.get(serial) || {};
  value.keyboard = cap.keyboard || "paste";
  value.name = cap.name || serial;
  return value;
}

function say(text, warm) {
  note.textContent = text || "";
  note.style.color = warm ? "var(--warm)" : "var(--dim)";
}

// --------------------------------------------------------------- cards ---

function audioReason(cap) {
  if (cap.audio === "none") {
    return `Android ${cap.release} cannot forward audio at all - scrcpy needs 11 or newer.`;
  }
  if (cap.audio === "output") {
    return `Android ${cap.release} only has the submix route, so sending audio here takes it off the handset.`;
  }
  return "";
}

function render() {
  const seen = new Set();
  for (const serial of order) {
    if (caps.has(serial)) {
      seen.add(serial);
      card(serial);
    }
  }
  for (const serial of caps.keys()) {
    if (!seen.has(serial)) {
      order.push(serial);
      card(serial);
    }
  }
  for (const node of [...rack.children]) {
    if (!caps.has(node.dataset.serial)) {
      node.remove();
    }
  }
  // only touch the DOM when the order really differs - moving nodes on every
  // poll makes the cards flicker and throws away accessibility handles
  const now = [...rack.children].map((node) => node.dataset.serial);
  const want = order.filter((serial) => caps.has(serial));
  if (now.join() !== want.join()) {
    for (const serial of want) {
      const node = rack.querySelector(`[data-serial="${serial}"]`);
      if (node) rack.appendChild(node);
    }
  }
  save();
}

function card(serial) {
  let node = rack.querySelector(`[data-serial="${serial}"]`);
  if (!node) {
    node = tmpl.content.firstElementChild.cloneNode(true);
    node.dataset.serial = serial;
    rack.appendChild(node);
    wire(node, serial);
  }
  paint(node, serial);
  return node;
}

function paint(node, serial) {
  const cap = caps.get(serial);
  const o = optionsFor(serial);
  const on = live.has(serial);

  const name = cap.name || serial;
  node.classList.toggle("live", on);
  node.querySelector(".name").textContent = name;
  // named for screen readers, which also makes every control addressable
  node.setAttribute("aria-label", name);
  const power = node.querySelector(".power");
  power.setAttribute("aria-label", `${on ? "stop" : "start"} ${name}`);
  for (const pad of node.querySelectorAll(".pads button")) {
    pad.setAttribute("aria-label", `${pad.dataset.act} ${name}`);
  }
  for (const seg of node.querySelectorAll(".seg")) {
    const what = seg.classList.contains("audio") ? "audio"
      : seg.classList.contains("size") ? "size" : "fps";
    for (const button of seg.querySelectorAll("button")) {
      button.setAttribute("aria-label", `${what} ${button.textContent.trim()} ${name}`);
    }
  }
  for (const flag of node.querySelectorAll(".flag")) {
    flag.setAttribute("aria-label", `${flag.dataset.flag} ${name}`);
  }

  const badges = node.querySelector(".badges");
  badges.innerHTML = "";
  const add = (text, cls) => {
    const b = document.createElement("b");
    b.textContent = text;
    if (cls) b.className = cls;
    badges.appendChild(b);
  };
  add(`A${cap.release}`);
  add(cap.keyboard, cap.keyboard === "uhid" ? "uhid" : "");
  if (typeof cap.battery === "number" && cap.battery >= 0) {
    add(`${cap.battery}%`, cap.battery <= 20 ? "low" : "");
  }

  for (const seg of node.querySelectorAll(".seg")) {
    const key = seg.classList.contains("audio") ? "audio"
      : seg.classList.contains("size") ? "max_size" : "max_fps";
    for (const button of seg.querySelectorAll("button")) {
      const value = key === "audio" ? button.dataset.value : Number(button.dataset.value);
      button.classList.toggle("on", o[key] === value);
      if (key === "audio") {
        button.disabled =
          (button.dataset.value === "output" && cap.audio === "none") ||
          (button.dataset.value === "dup" && cap.audio !== "dup");
      }
    }
  }

  for (const flag of node.querySelectorAll(".flag")) {
    flag.classList.toggle("on", !!o[flag.dataset.flag]);
  }

  node.querySelector(".why").textContent = o.audio !== "off" || cap.audio === "none"
    ? audioReason(cap)
    : "";
}

// -------------------------------------------------------------- wiring ---

function wire(node, serial) {
  node.querySelector(".power").addEventListener("click", (e) => {
    e.stopPropagation();
    live.has(serial) ? stop(serial) : start(serial);
  });

  for (const seg of node.querySelectorAll(".seg")) {
    const key = seg.classList.contains("audio") ? "audio"
      : seg.classList.contains("size") ? "max_size" : "max_fps";
    seg.addEventListener("click", (e) => {
      const button = e.target.closest("button");
      if (!button || button.disabled) return;
      const o = optionsFor(serial);
      o[key] = key === "audio" ? button.dataset.value : Number(button.dataset.value);
      save();
      paint(node, serial);
      restartIfLive(serial, key);
    });
  }

  for (const flag of node.querySelectorAll(".flag")) {
    flag.addEventListener("click", () => {
      const o = optionsFor(serial);
      const key = flag.dataset.flag;
      o[key] = !o[key];
      save();
      paint(node, serial);
      if (key === "skin" && live.has(serial)) {
        // the only toggle that can be applied to a running window
        invoke("set_skin", { serial, on: o.skin }).catch((err) => say(String(err), true));
        return;
      }
      restartIfLive(serial, key);
    });
  }

  node.querySelector(".pads").addEventListener("click", (e) => {
    const button = e.target.closest("button");
    if (!button) return;
    invoke("action", { serial, what: button.dataset.act })
      .then(() => say(`${button.dataset.act} sent`))
      .catch((err) => say(String(err), true));
  });

  node.querySelector(".grip").addEventListener("pointerdown", (e) => {
    if (e.target.closest(".power")) return;
    drag(node, e);
  });

  node.addEventListener("dblclick", () => {
    if (live.has(serial)) invoke("focus_device", { serial });
  });
}

async function restartIfLive(serial, key) {
  if (!live.has(serial)) return;
  say(`${key} needs a reconnect...`);
  await start(serial);
}

async function start(serial) {
  const node = card(serial);
  node.classList.add("busy");
  say("connecting...");
  try {
    await invoke("start", { serial, opts: optionsFor(serial) });
    live.add(serial);
    say("");
  } catch (err) {
    live.delete(serial);
    say(String(err), true);
  } finally {
    node.classList.remove("busy");
    paint(node, serial);
  }
}

async function stop(serial) {
  await invoke("stop", { serial });
  live.delete(serial);
  paint(card(serial), serial);
  say("");
}

// ------------------------------------------------------- dragging cards --

function drag(node, event) {
  const cards = [...rack.children];
  if (cards.length < 2) return;
  const startY = event.clientY;
  const home = node.getBoundingClientRect();
  node.classList.add("lifted");
  node.setPointerCapture(event.pointerId);

  const step = home.height + 10;

  const move = (e) => {
    const dy = e.clientY - startY;
    node.style.transform = `translateY(${dy}px) scale(1.02)`;
    const from = cards.indexOf(node);
    const to = Math.max(0, Math.min(cards.length - 1, from + Math.round(dy / step)));
    for (const [i, other] of cards.entries()) {
      if (other === node) continue;
      let shift = 0;
      if (from < to && i > from && i <= to) shift = -step;
      if (from > to && i < from && i >= to) shift = step;
      other.style.transform = shift ? `translateY(${shift}px)` : "";
    }
  };

  const drop = (e) => {
    node.releasePointerCapture(event.pointerId);
    node.removeEventListener("pointermove", move);
    node.removeEventListener("pointerup", drop);
    const dy = e.clientY - startY;
    const from = cards.indexOf(node);
    const to = Math.max(0, Math.min(cards.length - 1, from + Math.round(dy / step)));
    for (const other of cards) other.style.transform = "";
    node.classList.remove("lifted");
    if (to !== from) {
      order = cards.map((c) => c.dataset.serial);
      order.splice(to, 0, order.splice(from, 1)[0]);
      render();
      arrange();
    }
  };

  node.addEventListener("pointermove", move);
  node.addEventListener("pointerup", drop);
}

function arrange() {
  const running = order.filter((serial) => live.has(serial));
  if (running.length) {
    invoke("arrange", { order: running });
  }
}

// ---------------------------------------------------------------- boot ---

async function refresh() {
  try {
    const report = await invoke("probe");
    caps.clear();
    for (const device of report.devices) {
      caps.set(device.serial, device);
    }
    live = new Set(await invoke("running"));
    render();
    say(report.devices.length ? "" : "no phones on adb");
    for (const serial of caps.keys()) {
      invoke("status", { serial })
        .then((s) => {
          const cap = caps.get(serial);
          if (cap) {
            cap.battery = s.battery;
            const node = rack.querySelector(`[data-serial="${serial}"]`);
            if (node) paint(node, serial);
          }
        })
        .catch(() => {});
    }
  } catch (err) {
    say(String(err), true);
  }
}

document.getElementById("refresh").addEventListener("click", refresh);

document.getElementById("magnet").addEventListener("click", (e) => {
  const on = !e.target.classList.contains("on");
  e.target.classList.toggle("on", on);
  invoke("set_magnet", { on });
  say(on ? "windows snap to each other" : "magnet off");
});

document.getElementById("ontop").addEventListener("click", async (e) => {
  const on = !e.target.classList.contains("on");
  e.target.classList.toggle("on", on);
  await appWindow.setAlwaysOnTop(on);
});

document.getElementById("hide").addEventListener("click", () => invoke("hide_board"));

const loginChip = document.getElementById("login");
loginChip.addEventListener("click", async () => {
  try {
    const on = await invoke("set_autostart", { on: !loginChip.classList.contains("on") });
    loginChip.classList.toggle("on", on);
    say(on ? "starts with Windows, hidden in the tray" : "no longer starts with Windows");
  } catch (err) {
    say(String(err), true);
  }
});
invoke("autostart_state").then((on) => loginChip.classList.toggle("on", on));

// the tray can flip either of these behind the UI's back
listen("autostart-changed", (e) => loginChip.classList.toggle("on", !!e.payload));
listen("magnet-changed", (e) => {
  document.getElementById("magnet").classList.toggle("on", !!e.payload);
});

document.getElementById("arrange").addEventListener("click", arrange);
document.getElementById("logs").addEventListener("click", () => invoke("open_logs"));

listen("pedals-changed", (event) => {
  for (const serial of event.payload || []) {
    live.delete(serial);
    const node = rack.querySelector(`[data-serial="${serial}"]`);
    if (node) paint(node, serial);
  }
});

load();
refresh();
setInterval(refresh, 20000);
