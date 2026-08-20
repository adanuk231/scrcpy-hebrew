/* One set of controls, and a row of phones along the bottom deciding which
   phone they point at. Nothing here holds a picture: the phones are ordinary
   scrcpy windows out on the desktop. */

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const LogicalSize = window.__TAURI__.window.LogicalSize;

const deck = document.getElementById("deck");
const settings = document.getElementById("settings");
const tabsEl = document.getElementById("tabs");
const noteEl = document.getElementById("note");
const powerEl = document.getElementById("power");
const nameEl = deck.querySelector(".name");
const badgesEl = deck.querySelector(".badges");

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

/** serial -> what the phone can actually do, from the python probe */
const caps = new Map();
/** serial -> chosen options, remembered between runs */
const opts = new Map();
/** serials mirroring right now */
let live = new Set();
/** the order of the tabs, which is also the order "line them up" uses */
let order = [];
/** the phone every control on the deck points at */
let selected = null;

// ------------------------------------------------------------ settings ---

function load() {
  try {
    const raw = JSON.parse(localStorage.getItem("board") || "{}");
    order = raw.order || [];
    selected = raw.selected || null;
    for (const [serial, value] of Object.entries(raw.opts || {})) {
      opts.set(serial, { ...DEFAULTS, ...value });
    }
    return raw;
  } catch (_) {
    return {};
  }
}

function save() {
  localStorage.setItem("board", JSON.stringify({
    order,
    selected,
    onTop: document.body.dataset.onTop === "1",
    opts: Object.fromEntries(opts),
  }));
}

function optionsFor(serial) {
  if (!opts.has(serial)) {
    opts.set(serial, { ...DEFAULTS });
  }
  const value = opts.get(serial);
  const cap = caps.get(serial) || {};
  value.keyboard = cap.keyboard || "paste";
  value.name = cap.name || serial;
  return value;
}

/** A line in the header, whose height never changes, so nothing below it
    moves when the board has something to say. */
let noteTimer = null;

function say(text, kind, ms) {
  clearTimeout(noteTimer);
  noteEl.textContent = text || "";
  noteEl.title = text || "";
  noteEl.classList.toggle("warn", kind === "warn");
  if (text) {
    noteTimer = setTimeout(() => {
      noteEl.textContent = "";
      noteEl.title = "";
    }, ms || (kind === "warn" ? 9000 : 3500));
  }
}

// ---------------------------------------------------------- what it can --

/** Why an option is not available on this phone, or "" if it is. */
function blocked(cap, key, value) {
  if (key !== "audio") return "";
  if (value === "output" && cap.audio === "none") {
    return `${cap.name} runs Android ${cap.release}. scrcpy forwards audio from 11 up, so there is nothing to send.`;
  }
  if (value === "dup") {
    if (cap.audio === "none") {
      return `${cap.name} runs Android ${cap.release}, which has no audio forwarding at all.`;
    }
    if (cap.audio !== "dup") {
      return `Sharing needs Android 13 for playback capture. On ${cap.release} the only route is "to pc", and that takes the sound off the handset.`;
    }
  }
  return "";
}

/** What a control is worth knowing about on this phone, for its tooltip. */
function tooltip(cap, key, value) {
  const no = blocked(cap, key, value);
  if (no) return no;
  if (key === "audio" && value === "output") {
    return `Sends the sound here through REMOTE_SUBMIX, which takes it off ${cap.name} itself.`;
  }
  if (key === "audio" && value === "dup") {
    return "Captures playback and keeps the phone audible. Apps are allowed to opt out of being captured.";
  }
  return "";
}

// ---------------------------------------------------------------- deck ---

function paint() {
  const cap = caps.get(selected);
  const running = live.has(selected);
  document.body.classList.toggle("live", running);

  if (!cap) {
    nameEl.textContent = caps.size ? "pick a phone" : "no phones on adb";
    badgesEl.innerHTML = "";
    powerEl.disabled = true;
    paintTabs();
    return;
  }

  powerEl.disabled = false;
  nameEl.textContent = cap.name;
  powerEl.setAttribute("aria-label", `${running ? "stop" : "start"} ${cap.name}`);

  badgesEl.innerHTML = "";
  const add = (text, cls) => {
    const b = document.createElement("b");
    b.textContent = text;
    if (cls) b.className = cls;
    badgesEl.appendChild(b);
  };
  add(`A${cap.release}`);
  add(cap.keyboard, cap.keyboard === "uhid" ? "uhid" : "");
  badgesEl.lastChild.title = cap.keyboard === "uhid"
    ? `${cap.name} maps the keys itself, so Hebrew types straight through.`
    : `${cap.name} has no hardware keyboard layout, so Hebrew goes by clipboard.`;
  if (typeof cap.battery === "number" && cap.battery >= 0) {
    add(`${cap.battery}%`, cap.battery <= 20 ? "low" : "");
  }

  const o = optionsFor(selected);
  for (const seg of deck.querySelectorAll(".seg")) {
    const key = seg.dataset.key;
    for (const button of seg.querySelectorAll("button")) {
      const value = key === "audio" ? button.dataset.value : Number(button.dataset.value);
      button.classList.toggle("on", o[key] === value);
      // a phone that cannot do this gets a grey button and the reason on
      // hover, rather than a line of text that shifts everything below it
      button.disabled = !!blocked(cap, key, button.dataset.value);
      const why = tooltip(cap, key, button.dataset.value);
      if (why) {
        button.title = why;
      } else {
        button.removeAttribute("title");
      }
      button.setAttribute("aria-label", `${key} ${button.textContent.trim()} ${cap.name}`);
    }
  }
  for (const flag of deck.querySelectorAll(".flag")) {
    flag.classList.toggle("on", !!o[flag.dataset.flag]);
    flag.setAttribute("aria-label", `${flag.dataset.flag} ${cap.name}`);
  }
  for (const pad of deck.querySelectorAll(".pads button")) {
    pad.setAttribute("aria-label", `${pad.dataset.act} ${cap.name}`);
  }

  paintTabs();
  fit();
}

function paintTabs() {
  const want = order.filter((s) => caps.has(s));
  const have = [...tabsEl.children].map((n) => n.dataset.serial);
  if (want.join() !== have.join()) {
    tabsEl.innerHTML = "";
    for (const serial of want) {
      const tab = document.createElement("button");
      tab.className = "tab";
      tab.dataset.serial = serial;
      tab.innerHTML = '<span class="dot"></span>';
      tab.append(caps.get(serial).name);
      tabsEl.appendChild(tab);
    }
  }
  for (const tab of tabsEl.children) {
    const serial = tab.dataset.serial;
    tab.classList.toggle("on", serial === selected);
    tab.classList.toggle("running", live.has(serial));
    tab.setAttribute("aria-label", `select ${caps.get(serial).name}`);
  }
}

function select(serial) {
  selected = serial;
  showPage("deck");
  paint();
  save();
}

/* A panel should be exactly as tall as what is in it, so the window follows
   the content rather than leaving half of itself empty. */
let fitted = 0;
let fitting = null;

function fit() {
  clearTimeout(fitting);
  fitting = setTimeout(async () => {
    const header = document.querySelector("header.bar");
    const tabs = document.querySelector("nav.tabs");
    const page = settings.hidden ? deck : settings;
    const wanted = Math.max(300, Math.min(880, Math.ceil(
      header.offsetHeight + page.scrollHeight + tabs.offsetHeight + 2)));
    if (Math.abs(wanted - fitted) < 3) return;
    fitted = wanted;
    try {
      await appWindow.setSize(new LogicalSize(396, wanted));
      await invoke("settled");
    } catch (_) {
      /* the window may be on its way out */
    }
  }, 40);
}

function showPage(page) {
  deck.hidden = page !== "deck";
  settings.hidden = page !== "settings";
  document.getElementById("gear").classList.toggle("on", page === "settings");
  fit();
}

// ------------------------------------------------------------- actions ---

async function start(serial) {
  say("connecting...");
  try {
    await invoke("start", { serial, opts: optionsFor(serial) });
    live.add(serial);
    say("");
  } catch (err) {
    live.delete(serial);
    say(String(err), "warn");
  }
  paint();
}

async function stop(serial) {
  await invoke("stop", { serial });
  live.delete(serial);
  paint();
}

async function reconnect(serial, what) {
  if (!live.has(serial)) return;
  say(`${what} needs a reconnect...`);
  await start(serial);
}

powerEl.addEventListener("click", () => {
  if (!selected) return;
  live.has(selected) ? stop(selected) : start(selected);
});

for (const seg of deck.querySelectorAll(".seg")) {
  seg.addEventListener("click", (e) => {
    const button = e.target.closest("button");
    if (!button || !selected) return;
    if (button.disabled) return;
    const key = seg.dataset.key;
    const o = optionsFor(selected);
    o[key] = key === "audio" ? button.dataset.value : Number(button.dataset.value);
    save();
    paint();
    reconnect(selected, key);
  });
}

for (const flag of deck.querySelectorAll(".flag")) {
  flag.addEventListener("click", () => {
    if (!selected) return;
    const o = optionsFor(selected);
    const key = flag.dataset.flag;
    o[key] = !o[key];
    save();
    paint();
    if (key === "skin" && live.has(selected)) {
      // the one thing that can be changed on a window already open
      invoke("set_skin", { serial: selected, on: o.skin })
        .catch((err) => say(String(err), "warn"));
      return;
    }
    reconnect(selected, key);
  });
}

deck.querySelector(".pads").addEventListener("click", (e) => {
  const button = e.target.closest("button");
  if (!button || !selected) return;
  const name = caps.get(selected).name;
  invoke("action", { serial: selected, what: button.dataset.act })
    .then(() => say(`${button.dataset.act} sent to ${name}`, "", 1600))
    .catch((err) => say(String(err), "warn"));
});

// ------------------------------------------------------ tabs and header --

tabsEl.addEventListener("click", (e) => {
  const tab = e.target.closest(".tab");
  if (tab && !tab.dataset.dragged) select(tab.dataset.serial);
});

/** Drag a tab sideways to reorder; the phones follow when they are lined up. */
tabsEl.addEventListener("pointerdown", (e) => {
  const tab = e.target.closest(".tab");
  if (!tab || tabsEl.children.length < 2) return;
  const tabs = [...tabsEl.children];
  const startX = e.clientX;
  const step = tab.getBoundingClientRect().width + 6;
  let moved = false;
  tab.setPointerCapture(e.pointerId);

  const move = (ev) => {
    const dx = ev.clientX - startX;
    if (Math.abs(dx) > 4) moved = true;
    if (!moved) return;
    tab.classList.add("lifted");
    tab.style.transform = `translateX(${dx}px) scale(1.06)`;
  };

  const drop = (ev) => {
    tab.releasePointerCapture(e.pointerId);
    tabsEl.removeEventListener("pointermove", move);
    tabsEl.removeEventListener("pointerup", drop);
    tab.classList.remove("lifted");
    tab.style.transform = "";
    if (!moved) return;
    const from = tabs.indexOf(tab);
    const to = Math.max(0, Math.min(tabs.length - 1,
      from + Math.round((ev.clientX - startX) / step)));
    if (to !== from) {
      order = tabs.map((t) => t.dataset.serial);
      order.splice(to, 0, order.splice(from, 1)[0]);
      save();
      paintTabs();
      lineThemUp();
    }
    tab.dataset.dragged = "1";
    setTimeout(() => delete tab.dataset.dragged, 0);
  };

  tabsEl.addEventListener("pointermove", move);
  tabsEl.addEventListener("pointerup", drop);
});

function lineThemUp() {
  const running = order.filter((s) => live.has(s));
  if (running.length) invoke("arrange", { order: running });
}

document.getElementById("arrange").addEventListener("click", lineThemUp);
document.getElementById("close").addEventListener("click", () => invoke("hide_board"));
document.getElementById("gear").addEventListener("click", () => {
  showPage(settings.hidden ? "settings" : "deck");
});
document.getElementById("logs").addEventListener("click", () => invoke("open_logs"));

const loginBox = document.getElementById("login");
loginBox.addEventListener("change", async () => {
  try {
    loginBox.checked = await invoke("set_autostart", { on: loginBox.checked });
  } catch (err) {
    loginBox.checked = false;
    say(String(err), "warn");
  }
});

const ontopBox = document.getElementById("ontop");
ontopBox.addEventListener("change", async () => {
  document.body.dataset.onTop = ontopBox.checked ? "1" : "0";
  await appWindow.setAlwaysOnTop(ontopBox.checked);
  save();
});

/* Keys, because a panel that lives in the tray is often the only thing on
   screen: Escape puts it away, comma opens the settings behind the gear, and
   the number keys pick a phone. */
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    invoke("hide_board");
    return;
  }
  if (e.key === "," || (e.ctrlKey && e.key === ",")) {
    showPage(settings.hidden ? "settings" : "deck");
    return;
  }
  const n = Number(e.key);
  if (n >= 1 && n <= 9 && order[n - 1]) {
    select(order[n - 1]);
  }
});

// ---------------------------------------------------------------- boot ---

async function refresh() {
  try {
    const report = await invoke("probe");
    caps.clear();
    for (const device of report.devices) caps.set(device.serial, device);
    for (const serial of caps.keys()) {
      if (!order.includes(serial)) order.push(serial);
    }
    order = order.filter((s) => caps.has(s));
    live = new Set(await invoke("running"));
    if (!caps.has(selected)) selected = order[0] || null;
    paint();
    save();

    for (const serial of caps.keys()) {
      invoke("status", { serial })
        .then((s) => {
          const cap = caps.get(serial);
          if (cap) {
            cap.battery = s.battery;
            if (serial === selected) paint();
          }
        })
        .catch(() => {});
    }
  } catch (err) {
    say(String(err), "warn");
  }
}

document.getElementById("refresh").addEventListener("click", refresh);

listen("pedals-changed", (event) => {
  for (const serial of event.payload || []) live.delete(serial);
  paint();
});

listen("detached", (event) => {
  const cap = caps.get(event.payload);
  if (cap) say(`${cap.name} taken out of its group`, "", 2200);
});

listen("autostart-changed", (e) => { loginBox.checked = !!e.payload; });

const saved = load();
showPage("deck");
invoke("autostart_state").then((on) => { loginBox.checked = on; });
const onTop = saved.onTop !== false;
ontopBox.checked = onTop;
document.body.dataset.onTop = onTop ? "1" : "0";
appWindow.setAlwaysOnTop(onTop);
refresh();
setInterval(refresh, 20000);
