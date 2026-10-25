// Hormuz UI. Runs inside the Tauri app (desktop + mobile) and as the daemon's web page.
// Both expose the same command set: Tauri via invoke("rpc"), the daemon via POST /rpc/<cmd>.

const tauri = window.__TAURI__;
const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

async function call(cmd, args = {}) {
  if (tauri) return tauri.core.invoke("rpc", { cmd, args });
  const res = await fetch(`/rpc/${cmd}`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-hormuz": "1" },
    body: JSON.stringify(args),
  });
  const body = await res.json().catch(() => ({ error: res.statusText }));
  if (!res.ok) throw new Error(body.error || res.statusText);
  return body;
}

function openUrl(url) {
  if (tauri?.opener) tauri.opener.openUrl(url);
  else window.open(url, "_blank", "noopener");
}

const $ = (sel) => document.querySelector(sel);
const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

function fmtDays(days) {
  if (!days.length) return "Every day";
  if (days.join() === "0,1,2,3,4") return "Weekdays";
  if (days.join() === "5,6") return "Weekends";
  return days.map((d) => DAYS[d]).join(", ");
}

function hhmm(iso) {
  return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

let toastTimer;
function toast(msg) {
  const t = $("#toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), 2600);
}

async function guard(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(String(e.message ?? e));
  }
}

// ---------- state ----------

let tasks = [];
let view = "today";

async function refresh() {
  const [agenda, all] = await Promise.all([call("agenda"), call("list_tasks")]);
  tasks = all;
  renderAgenda(agenda);
  renderTasks(all);
}

// ---------- today ----------

function renderAgenda(a) {
  const date = new Date(a.date + "T12:00:00");
  $("#date").textContent = date.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
  $("#count").textContent = `${a.done}/${a.total}`;
  $("#ring").style.strokeDasharray = `${a.total ? (a.done / a.total) * 100 : 0} 100`;
  $("#empty-today").hidden = a.items.length > 0;

  const now = new Date();
  $("#agenda").innerHTML = a.items
    .map(({ task: t, done, done_at, streak, next_time, overdue }) => {
      const snoozed = t.snoozed_until && new Date(t.snoozed_until) > now;
      const chips = [];
      if (done) chips.push(`<span class="chip good">Done ${hhmm(done_at)}</span>`);
      else if (snoozed) chips.push(`<span class="chip">Snoozed till ${hhmm(t.snoozed_until)}</span>`);
      else if (overdue) chips.push(`<span class="chip warn">Due since ${esc(t.times[0])}</span>`);
      else chips.push(`<span class="chip">${esc(t.times.join(" · ") || "Any time")}</span>`);
      if (!done && overdue && next_time) chips.push(`<span class="chip">Next ${esc(next_time)}</span>`);
      if (streak > 1) chips.push(`<span class="chip good" title="Days in a row">🔥 ${streak}</span>`);
      const link = t.url ? `<button class="link-btn" data-open="${esc(t.url)}">Open ↗</button>` : "";
      const snooze = done
        ? ""
        : `<details class="snooze"><summary class="ghost">Snooze</summary><div class="menu">
             <button class="ghost" data-snooze="${t.id}" data-min="15">15 minutes</button>
             <button class="ghost" data-snooze="${t.id}" data-min="60">1 hour</button>
             <button class="ghost" data-snooze="${t.id}" data-min="180">3 hours</button>
             ${snoozed ? `<button class="ghost" data-snooze="${t.id}" data-min="0">Unsnooze</button>` : ""}
           </div></details>`;
      return `<li class="item ${done ? "done" : ""} ${overdue && !snoozed ? "overdue" : ""}">
        <button class="tick" data-tick="${t.id}" data-done="${done}" aria-label="${done ? "Mark not done" : "Mark done"}">✓</button>
        <div class="title">${esc(t.title)}</div>
        ${t.notes ? `<div class="notes">${esc(t.notes)}</div>` : ""}
        <div class="meta">${chips.join("")}${link}${snooze}</div>
      </li>`;
    })
    .join("");
}

// ---------- routine ----------

function renderTasks(all) {
  $("#empty-tasks").hidden = all.length > 0;
  $("#tasks").innerHTML = all
    .map(
      (t) => `<li class="item ${t.enabled ? "" : "off"}">
        <span class="tick" aria-hidden="true" style="visibility:hidden"></span>
        <div class="title">${esc(t.title)}</div>
        ${t.notes ? `<div class="notes">${esc(t.notes)}</div>` : ""}
        <div class="meta">
          <span class="chip">${esc(t.times.join(" · ") || "No reminder")}</span>
          <span class="chip">${fmtDays(t.days)}</span>
          ${t.nag_minutes ? `<span class="chip">Nag every ${t.nag_minutes}m</span>` : ""}
          ${t.enabled ? "" : `<span class="chip warn">Off</span>`}
          <button class="link-btn" data-edit="${t.id}">Edit</button>
        </div>
      </li>`,
    )
    .join("");
}

// ---------- editor ----------

let editing = null;
let pickedDays = new Set();

function timeRow(value) {
  const div = document.createElement("div");
  div.className = "time";
  div.innerHTML = `<input type="time" required value="${esc(value)}"><button type="button" class="ghost" aria-label="Remove time">✕</button>`;
  div.querySelector("button").onclick = () => {
    if ($("#times").children.length > 1) div.remove();
  };
  return div;
}

function renderDays() {
  $("#days").innerHTML = DAYS.map(
    (d, i) => `<button type="button" data-day="${i}" aria-pressed="${pickedDays.size === 0 || pickedDays.has(i)}">${d.slice(0, 2)}</button>`,
  ).join("");
}

function openEditor(task) {
  editing = task;
  const f = $("#form");
  f.reset();
  $("#form-error").hidden = true;
  $("#form-title").textContent = task ? "Edit task" : "New task";
  $("#delete").hidden = !task;
  f.title.value = task?.title ?? "";
  f.url.value = task?.url ?? "";
  f.notes.value = task?.notes ?? "";
  f.nag.value = task?.nag_minutes ?? "";
  f.enabled.checked = task?.enabled ?? true;
  $("#times").replaceChildren(...(task?.times?.length ? task.times : ["09:00"]).map(timeRow));
  pickedDays = new Set(task?.days ?? []);
  renderDays();
  $("#editor").showModal();
  f.title.focus();
}

$("#days").addEventListener("click", (e) => {
  const b = e.target.closest("[data-day]");
  if (!b) return;
  const i = Number(b.dataset.day);
  // Empty set means "every day"; expand it before toggling one off.
  if (pickedDays.size === 0) pickedDays = new Set(DAYS.map((_, j) => j));
  pickedDays.has(i) ? pickedDays.delete(i) : pickedDays.add(i);
  if (pickedDays.size === 0) pickedDays.add(i); // at least one day
  if (pickedDays.size === 7) pickedDays.clear();
  renderDays();
});

$("#add-time").onclick = () => $("#times").append(timeRow("18:00"));
$("#cancel").onclick = () => $("#editor").close();

$("#form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = e.target;
  const input = {
    title: f.title.value.trim(),
    url: f.url.value.trim() || null,
    notes: f.notes.value.trim(),
    times: [...$("#times").querySelectorAll("input")].map((i) => i.value).filter(Boolean),
    days: [...pickedDays].sort(),
    nag_minutes: f.nag.value ? Number(f.nag.value) : null,
    enabled: f.enabled.checked,
  };
  try {
    if (editing) await call("update_task", { id: editing.id, input });
    else await call("add_task", { input });
    $("#editor").close();
    toast(editing ? "Saved" : "Task added");
    await refresh();
  } catch (err) {
    $("#form-error").textContent = String(err.message ?? err);
    $("#form-error").hidden = false;
  }
});

$("#delete").onclick = () =>
  guard(async () => {
    if (!editing || !confirm(`Delete “${editing.title}” and its history?`)) return;
    await call("delete_task", { id: editing.id });
    $("#editor").close();
    toast("Deleted");
    await refresh();
  });

// ---------- events ----------

document.addEventListener("click", (e) => {
  const el = e.target.closest("[data-tick],[data-open],[data-snooze],[data-edit],[data-action],[data-view]");
  if (!el) return;
  const d = el.dataset;
  if (d.tick)
    guard(async () => {
      await call("set_done", { id: Number(d.tick), done: d.done !== "true" });
      await refresh();
    });
  else if (d.open) openUrl(d.open);
  else if (d.snooze)
    guard(async () => {
      await call("snooze", { id: Number(d.snooze), minutes: Number(d.min) });
      toast(Number(d.min) ? `Snoozed ${d.min >= 60 ? d.min / 60 + "h" : d.min + "m"}` : "Unsnoozed");
      await refresh();
    });
  else if (d.edit) openEditor(tasks.find((t) => t.id === Number(d.edit)));
  else if (d.action === "seed")
    guard(async () => {
      const added = await call("seed_defaults");
      toast(added.length ? `Added ${added.length} tasks` : "Starter tasks already there");
      await refresh();
    });
  else if (d.view) setView(d.view);
});

// Close an open snooze menu when clicking elsewhere.
document.addEventListener("click", (e) => {
  document.querySelectorAll("details.snooze[open]").forEach((m) => !m.contains(e.target) && m.removeAttribute("open"));
});

$("#add").onclick = () => openEditor(null);

function setView(v) {
  view = v;
  document.querySelectorAll(".tabs button").forEach((b) => b.setAttribute("aria-selected", b.dataset.view === v));
  $("#view-today").hidden = v !== "today";
  $("#view-tasks").hidden = v !== "tasks";
}

async function showStatus() {
  if (!tauri) {
    $("#status").textContent = "Served by the hormuz daemon on this machine.";
    return;
  }
  const info = await call("host_info").catch(() => null);
  if (!info) return;
  $("#status").textContent = info.mobile
    ? "Reminders are scheduled on this device."
    : info.daemon
      ? "Background daemon is running: reminders arrive even after you quit Hormuz."
      : "Reminders only fire while Hormuz is running (closing the window keeps it in the menu bar). Run `hormuz daemon install` to get them even after you quit.";
}

guard(refresh);
showStatus();
setInterval(() => guard(refresh), 30_000);
document.addEventListener("visibilitychange", () => !document.hidden && guard(refresh));
// Ticking a task in the menu bar menu redraws the window straight away.
tauri?.event?.listen("tasks-changed", () => guard(refresh));
