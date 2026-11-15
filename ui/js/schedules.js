/* Jobs tab: the Schedules, Rules and Next 24 hours views, and the buttons
   that switch between them and the Queue view (js/jobs.js).

   Schedules and rules live in jobs.json in the settings folder and are run
   by src-tauri/src/scheduler.rs. This page adds, changes and deletes them
   through commands that check each entry in Rust before writing the file,
   so nothing invalid is saved; the error is shown under the form instead.
   Rust pushes the whole list as a "schedules:changed" window event after
   every change (a run started or finished, a save, a reload). */
(function () {
  "use strict";

  var tauri = window.__TAURI__;
  var invoke = tauri.core.invoke;

  var STATES = {
    queued: { label: "Queued", badge: "badge-neutral" },
    running: { label: "Running", badge: "badge-info" },
    done: { label: "Done", badge: "badge-success" },
    failed: { label: "Failed", badge: "badge-danger" },
    cancelled: { label: "Killed", badge: "badge-warning" },
    timed_out: { label: "Timed out", badge: "badge-danger" }
  };

  var CONFIRM_MS = 4000; // second click on Delete must come within this

  function $(id) { return document.getElementById(id); }

  var tab = $("tab-jobs");
  var banner = $("sched-paused-banner");
  var pauseAllButton = $("btn-sched-pause-all");
  var schedStatus = $("sched-status");
  var skippedBox = $("sched-skipped");
  var schedRows = $("sched-rows");
  var ruleRows = $("rule-rows");
  var ruleStatus = $("rule-status");
  var upcomingRows = $("upcoming-rows");
  var historyCard = $("sched-history");
  var historyRows = $("sched-history-rows");

  var sEditor = $("sched-editor");
  var f = {
    name: $("sched-name"), when: $("sched-when"), at: $("sched-at"),
    every: $("sched-every"), everyUnit: $("sched-every-unit"), weekday: $("sched-weekday"),
    monthday: $("sched-monthday"), time: $("sched-time"),
    maxRuns: $("sched-max-runs"), until: $("sched-until"), timeout: $("sched-timeout"),
    retries: $("sched-retries"), retryWait: $("sched-retry-wait"), retryUnit: $("sched-retry-unit"),
    enabled: $("sched-enabled"), catchUp: $("sched-catch-up"), doubling: $("sched-doubling"),
    note: $("sched-note"), error: $("sched-editor-error"), title: $("sched-editor-title")
  };
  var rEditor = $("rule-editor");
  var r = {
    name: $("rule-name"), source: $("rule-source"), ended: $("rule-ended"), label: $("rule-label"),
    times: $("rule-times"), action: $("rule-action"), target: $("rule-target"),
    field: $("rule-value-field"), above: $("rule-value-above"), below: $("rule-value-below"), equals: $("rule-value-equals"),
    changed: $("rule-changed"),
    enabled: $("rule-enabled"), note: $("rule-note"), error: $("rule-editor-error"), title: $("rule-editor-title")
  };

  var jobFields = window.JobForms.mount($("sched-job-fields"), "sched-job-");
  var reply = null;      // the last SchedulesReply from Rust
  var editingSched = null; // name being edited, null for a new one
  var editingRule = null;
  var historyOf = null;  // schedule whose history is shown

  // ---- Small helpers ---------------------------------------------------------

  function el(tag, className, text) {
    var e = document.createElement(tag);
    if (className) { e.className = className; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  function badge(kind, text) {
    return el("span", "badge " + kind, text);
  }

  function button(icon, label, action) {
    var b = el("button", "btn btn-secondary btn-sm");
    b.type = "button";
    b.dataset.action = action;
    b.setAttribute("aria-label", label);
    b.title = label;
    b.appendChild(el("i", "bi " + icon));
    return b;
  }

  function pad(n) { return (n < 10 ? "0" : "") + n; }

  // "today 14:05:30", "tomorrow 08:00", "Mon 1 Dec 09:00".
  function when(ms) {
    if (ms === null || ms === undefined) { return "–"; }
    var d = new Date(ms);
    var now = new Date();
    var tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
    var time = pad(d.getHours()) + ":" + pad(d.getMinutes());
    if (d.toDateString() === now.toDateString()) { return "today " + time + ":" + pad(d.getSeconds()); }
    if (d.toDateString() === tomorrow.toDateString()) { return "tomorrow " + time; }
    var opts = { weekday: "short", day: "numeric", month: "short" };
    if (d.getFullYear() !== now.getFullYear()) { opts.year = "numeric"; }
    return d.toLocaleDateString("en-GB", opts) + " " + time;
  }

  // The same format jobs.json uses: "2026-12-01 09:00".
  function plainDate(d) {
    return d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate()) + " " +
      pad(d.getHours()) + ":" + pad(d.getMinutes());
  }

  // "90m" -> [90, "m"]; anything else -> null.
  function splitInterval(text) {
    var m = /^\s*(\d+)\s*([smhd])\s*$/.exec(text || "");
    return m ? [Number(m[1]), m[2]] : null;
  }

  function resultBadge(state, label) {
    var span = el("span");
    if (!state) { span.textContent = "–"; return span; }
    var s = STATES[state] || STATES.failed;
    span.appendChild(badge(s.badge, s.label));
    if (label) { span.appendChild(el("span", "jobs-subtle", label)); }
    return span;
  }

  function emptyRow(tbody, columns, text) {
    var tr = el("tr");
    var td = el("td", "jobs-empty", text);
    td.colSpan = columns;
    tr.appendChild(td);
    tbody.appendChild(tr);
  }

  function setStatus(target, text) { target.textContent = text; }

  function schedule(name) {
    return reply ? reply.schedules.filter(function (s) { return s.name === name; })[0] : null;
  }

  // Delete needs a second click within CONFIRM_MS.
  function confirmed(b) {
    if (b.classList.contains("jobs-confirm")) { return true; }
    b.classList.add("jobs-confirm");
    var label = b.getAttribute("aria-label");
    b.title = "Click again to delete";
    setTimeout(function () {
      b.classList.remove("jobs-confirm");
      b.title = label;
    }, CONFIRM_MS);
    return false;
  }

  // ---- Views -----------------------------------------------------------------

  var viewButtons = document.querySelectorAll("[data-jobs-view]");
  var panes = document.querySelectorAll("[data-jobs-pane]");

  function showView(name) {
    viewButtons.forEach(function (b) {
      var on = b.dataset.jobsView === name;
      b.classList.toggle("active", on);
      b.setAttribute("aria-pressed", on ? "true" : "false");
    });
    panes.forEach(function (p) { p.classList.toggle("d-none", p.dataset.jobsPane !== name); });
  }

  viewButtons.forEach(function (b) {
    b.addEventListener("click", function () { showView(b.dataset.jobsView); });
  });

  // ---- Rendering ----------------------------------------------------------------

  // The second line of the When cell: what happens next.
  function nextLine(s) {
    var line = el("div", "jobs-subtle");
    if (s.running_job) {
      line.appendChild(badge("badge-info", "Running #" + s.running_job));
    } else if (s.paused) {
      line.appendChild(badge("badge-warning", "Paused"));
      if (s.paused_reason) { line.appendChild(document.createTextNode(" " + s.paused_reason)); }
    } else if (!s.enabled) {
      line.appendChild(badge("badge-neutral", "Off"));
    } else if (s.retry_due_ms) {
      line.textContent = "retry " + when(s.retry_due_ms);
    } else if (s.next_ms) {
      line.textContent = "next " + when(s.next_ms);
    } else if (s.when.indexOf("manual") !== 0) {
      line.textContent = "no more runs";
    }
    return line;
  }

  function renderSchedules() {
    schedRows.textContent = "";
    if (!reply.schedules.length) {
      emptyRow(schedRows, 5, "No schedules yet. Add one above.");
    }
    reply.schedules.forEach(function (s) {
      var tr = el("tr");
      tr.dataset.name = s.name;
      var name = el("td");
      name.appendChild(el("div", "", s.name));
      name.appendChild(el("div", "jobs-subtle", window.JobForms.label(s.kind) + (s.definition && s.definition.note ? " · " + s.definition.note : "")));
      tr.appendChild(name);
      var w = el("td", "jobs-wrap", s.when);
      w.appendChild(nextLine(s));
      if (s.enabled !== s.enabled_in_file) { w.appendChild(el("div", "jobs-subtle", "turned " + (s.enabled ? "on" : "off") + " by a rule")); }
      tr.appendChild(w);
      tr.appendChild(el("td", "jobs-number jobs-narrow-hide", s.runs + (s.max_runs ? " / " + s.max_runs : "")));
      var last = el("td", "jobs-extra");
      last.appendChild(resultBadge(s.last_state, s.last_label));
      tr.appendChild(last);
      var cell = el("td", "jobs-buttons");
      var b = el("div", "jobs-button-row");
      cell.appendChild(b);
      b.appendChild(button("bi-play-fill", "Run " + s.name + " now", "run"));
      b.appendChild(button(s.enabled_in_file ? "bi-toggle-on" : "bi-toggle-off",
        (s.enabled_in_file ? "Turn off " : "Turn on ") + s.name, "toggle"));
      if (s.paused) { b.appendChild(button("bi-arrow-repeat", "Resume " + s.name, "resume")); }
      b.appendChild(button("bi-pencil", "Edit " + s.name, "edit"));
      b.appendChild(button("bi-clock-history", "History of " + s.name, "history"));
      tr.appendChild(cell);
      schedRows.appendChild(tr);
    });
  }

  function renderRules() {
    ruleRows.textContent = "";
    if (!reply.rules.length) {
      emptyRow(ruleRows, 4, reply.schedules.length ? "No rules yet. Add one above." : "Add a schedule first; rules connect schedules.");
    }
    reply.rules.forEach(function (rule) {
      var tr = el("tr");
      tr.dataset.name = rule.name;
      var name = el("td", "", rule.name);
      if (!rule.enabled) { name.appendChild(el("span", "jobs-subtle", " ")); name.appendChild(badge("badge-neutral", "Off")); }
      tr.appendChild(name);
      var text = el("td", "jobs-wrap", rule.text);
      if (rule.definition && rule.definition.note) { text.appendChild(el("div", "jobs-subtle", rule.definition.note)); }
      tr.appendChild(text);
      tr.appendChild(el("td", "jobs-number jobs-narrow-hide", rule.streak + " / " + rule.times));
      var b = el("td", "jobs-buttons");
      b.appendChild(button("bi-pencil", "Edit " + rule.name, "edit"));
      b.appendChild(button("bi-trash", "Delete " + rule.name, "delete"));
      tr.appendChild(b);
      ruleRows.appendChild(tr);
    });
    $("rule-max-chain").textContent = reply.max_chain;
  }

  function renderUpcoming() {
    upcomingRows.textContent = "";
    if (reply.paused_all) {
      emptyRow(upcomingRows, 3, "Everything is paused, so nothing will run.");
      return;
    }
    if (!reply.upcoming.length) {
      emptyRow(upcomingRows, 3, "Nothing is due in the next 24 hours.");
    }
    reply.upcoming.forEach(function (u) {
      var tr = el("tr");
      tr.appendChild(el("td", "jobs-number", when(u.at_ms)));
      tr.appendChild(el("td", "", u.name));
      tr.appendChild(el("td", "", u.why === "retry" ? "retry after a failure" : "timetable"));
      upcomingRows.appendChild(tr);
    });
  }

  function renderHistory() {
    var s = historyOf && schedule(historyOf);
    if (!s) { historyCard.classList.add("d-none"); historyOf = null; return; }
    $("sched-history-title").textContent = "History of " + s.name + " (newest first)";
    historyRows.textContent = "";
    var list = (s.history || []).slice().reverse();
    if (!list.length) { emptyRow(historyRows, 5, "No finished runs yet."); }
    list.forEach(function (h) {
      var tr = el("tr");
      tr.appendChild(el("td", "jobs-number", when(h.at_ms)));
      var res = el("td");
      res.appendChild(resultBadge(h.state, h.label));
      if (h.value !== undefined && h.value !== null) {
        res.appendChild(el("div", "jobs-subtle jobs-history-value", JSON.stringify(h.value)));
      }
      tr.appendChild(res);
      tr.appendChild(el("td", "", h.by));
      tr.appendChild(el("td", "jobs-number", h.job ? "#" + h.job : "not started"));
      var out = el("td", "jobs-wrap jobs-history-output");
      if (h.output) {
        out.appendChild(el("pre", "jobs-output jobs-output-small", h.output.trim()));
      } else {
        out.textContent = "–";
      }
      tr.appendChild(out);
      historyRows.appendChild(tr);
    });
    historyCard.classList.remove("d-none");
  }

  function render() {
    if (!reply) { return; }
    banner.classList.toggle("d-none", !reply.paused_all);
    pauseAllButton.querySelector("span").textContent = reply.paused_all ? "Resume everything" : "Pause everything";
    pauseAllButton.querySelector("i").className = "bi me-1 " + (reply.paused_all ? "bi-play-circle" : "bi-pause-circle");
    pauseAllButton.setAttribute("aria-pressed", reply.paused_all ? "true" : "false");
    $("sched-path").textContent = reply.path;
    skippedBox.textContent = "";
    if (reply.skipped.length) {
      skippedBox.appendChild(el("div", "", "Skipped in jobs.json (fix it there, then press Reload):"));
      reply.skipped.forEach(function (text) { skippedBox.appendChild(el("div", "", "• " + text)); });
    }
    skippedBox.classList.toggle("d-none", !reply.skipped.length);
    renderSchedules();
    renderRules();
    renderUpcoming();
    renderHistory();
  }

  function load() {
    return invoke("schedules_list").then(function (r2) {
      reply = r2;
      render();
    }).catch(function (err) {
      setStatus(schedStatus, "Could not load the schedules: " + err);
    });
  }

  tauri.event.listen("schedules:changed", function (e) {
    reply = e.payload;
    render();
  });

  // ---- Schedule editor --------------------------------------------------------------

  function showWhenFields() {
    var type = f.when.value;
    sEditor.querySelectorAll("[data-when]").forEach(function (box) {
      box.classList.toggle("d-none", box.dataset.when.split(" ").indexOf(type) < 0);
    });
  }

  function setIntervalFields(numberInput, unitSelect, text, fallback) {
    var parts = splitInterval(text) || fallback;
    numberInput.value = parts[0];
    unitSelect.value = parts[1];
  }

  function openScheduleEditor(name) {
    var s = name ? schedule(name) : null;
    var d = s ? s.definition : {};
    var w = d.when || { type: "daily", time: "08:00" };
    editingSched = s ? s.name : null;
    f.title.textContent = s ? "Edit " + s.name : "New schedule";
    jobFields.set(d.kind, d.input);
    f.name.value = d.name || "";
    f.when.value = w.type;
    var tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(9, 0, 0, 0);
    f.at.value = w.at || plainDate(tomorrow);
    setIntervalFields(f.every, f.everyUnit, w.interval, [1, "h"]);
    f.weekday.value = w.day && typeof w.day === "string" ? w.day.toLowerCase() : "monday";
    f.monthday.value = typeof w.day === "number" ? w.day : 1;
    f.time.value = w.time || "08:00";
    f.maxRuns.value = d.max_runs || "";
    f.until.value = d.until || "";
    f.timeout.value = d.timeout_ms ? Math.round(d.timeout_ms / 1000) : "";
    f.retries.value = d.retries || 0;
    setIntervalFields(f.retryWait, f.retryUnit, d.retry_wait, [1, "m"]);
    f.enabled.checked = d.enabled !== false;
    f.catchUp.checked = d.catch_up !== false;
    f.doubling.checked = d.retry_doubling !== false;
    f.note.value = d.note || "";
    f.error.textContent = "";
    $("btn-sched-delete").classList.toggle("d-none", !s);
    showWhenFields();
    sEditor.classList.remove("d-none");
    f.name.focus();
    sEditor.scrollIntoView({ block: "nearest" });
  }

  function closeScheduleEditor() {
    sEditor.classList.add("d-none");
    editingSched = null;
  }

  function scheduleFromForm() {
    var job = jobFields.read();
    var w = { type: f.when.value };
    if (w.type === "once") { w.at = f.at.value.trim(); }
    if (w.type === "every") { w.interval = f.every.value + f.everyUnit.value; }
    if (w.type === "weekly") { w.day = f.weekday.value; }
    if (w.type === "monthly") { w.day = Number(f.monthday.value); }
    if (["daily", "weekly", "monthly"].indexOf(w.type) >= 0) { w.time = f.time.value.trim(); }
    var d = { name: f.name.value.trim(), kind: job.kind, input: job.input, when: w, enabled: f.enabled.checked };
    if (!f.catchUp.checked) { d.catch_up = false; }
    if (f.maxRuns.value !== "") { d.max_runs = Number(f.maxRuns.value); }
    if (f.until.value.trim() !== "") { d.until = f.until.value.trim(); }
    if (f.timeout.value !== "") { d.timeout_ms = Number(f.timeout.value) * 1000; }
    var retries = Number(f.retries.value || 0);
    if (retries > 0) {
      d.retries = retries;
      d.retry_wait = f.retryWait.value + f.retryUnit.value;
      if (!f.doubling.checked) { d.retry_doubling = false; }
    }
    if (f.note.value.trim() !== "") { d.note = f.note.value.trim(); }
    return d;
  }

  f.when.addEventListener("change", showWhenFields);

  sEditor.addEventListener("submit", function (e) {
    e.preventDefault();
    var d;
    try {
      d = scheduleFromForm();
    } catch (err) {
      f.error.textContent = "Not saved: " + err.message + ".";
      return;
    }
    invoke("schedules_save", { oldName: editingSched, definition: d }).then(function (r2) {
      reply = r2;
      closeScheduleEditor();
      render();
      setStatus(schedStatus, "Saved " + d.name + ".");
    }, function (err) {
      f.error.textContent = "Not saved: " + err + ".";
    });
  });

  $("btn-sched-cancel").addEventListener("click", closeScheduleEditor);
  $("btn-sched-add").addEventListener("click", function () { openScheduleEditor(null); });

  $("btn-sched-reload").addEventListener("click", function () {
    invoke("schedules_reload").then(function (r2) {
      reply = r2;
      render();
      setStatus(schedStatus, "Read jobs.json again: " + r2.schedules.length + " schedules, " + r2.rules.length + " rules" +
        (r2.skipped.length ? ", " + r2.skipped.length + " skipped." : "."));
    });
  });

  pauseAllButton.addEventListener("click", function () {
    var pause = !(reply && reply.paused_all);
    invoke("schedules_pause_all", { paused: pause }).then(function (r2) {
      reply = r2;
      render();
      setStatus(schedStatus, pause ? "Everything paused." : "Resumed. Runs missed while paused are not made up.");
    });
  });

  schedRows.addEventListener("click", function (e) {
    var b = e.target.closest("button[data-action]");
    if (!b) { return; }
    var name = b.closest("tr").dataset.name;
    var s = schedule(name);
    var done = function (text) { return function () { setStatus(schedStatus, text); }; };
    var failed = function (err) { setStatus(schedStatus, name + ": " + err + "."); };
    switch (b.dataset.action) {
      case "run":
        invoke("schedules_run_now", { name: name }).then(function (id) {
          setStatus(schedStatus, "Started " + name + " as job #" + id + " (see Queue).");
        }, failed);
        break;
      case "toggle":
        invoke("schedules_set_enabled", { name: name, enabled: !s.enabled_in_file })
          .then(done((s.enabled_in_file ? "Turned off " : "Turned on ") + name + "."), failed);
        break;
      case "resume":
        invoke("schedules_pause", { name: name, paused: false }).then(done("Resumed " + name + "."), failed);
        break;
      case "edit":
        openScheduleEditor(name);
        break;
      case "history":
        historyOf = name;
        renderHistory();
        historyCard.scrollIntoView({ block: "nearest" });
        break;
    }
  });

  // Delete is in the edit form, so it cannot be clicked by mistake in the
  // table, and still needs a second click.
  var schedDelete = $("btn-sched-delete");
  schedDelete.addEventListener("click", function () {
    var name = editingSched;
    if (!name) { return; }
    if (!confirmed(schedDelete)) { f.error.textContent = "Click Delete again to delete " + name + "."; return; }
    invoke("schedules_delete", { name: name }).then(function (r2) {
      reply = r2;
      closeScheduleEditor();
      render();
      setStatus(schedStatus, "Deleted " + name + ".");
    }, function (err) {
      f.error.textContent = "Not deleted: " + err + ".";
    });
  });

  $("btn-sched-history-close").addEventListener("click", function () {
    historyOf = null;
    historyCard.classList.add("d-none");
  });

  // ---- Rule editor ------------------------------------------------------------------

  function fillScheduleSelect(select, value) {
    select.textContent = "";
    reply.schedules.forEach(function (s) {
      var o = el("option", "", s.name);
      o.value = s.name;
      select.appendChild(o);
    });
    if (value) { select.value = value; }
  }

  function openRuleEditor(name) {
    if (!reply.schedules.length) {
      setStatus(ruleStatus, "Add a schedule first: a rule connects schedules.");
      return;
    }
    var rule = name ? reply.rules.filter(function (x) { return x.name === name; })[0] : null;
    var d = rule ? rule.definition : {};
    var w = d.when || {};
    var t = d.then || {};
    editingRule = rule ? rule.name : null;
    r.title.textContent = rule ? "Edit " + rule.name : "New rule";
    r.name.value = d.name || "";
    fillScheduleSelect(r.source, w.schedule);
    fillScheduleSelect(r.target, t.schedule);
    r.ended.value = w.ended || "failed";
    r.label.value = w.label || "";
    r.times.value = w.times || 1;
    var v = w.value || {};
    r.field.value = v.field || "";
    r.above.value = v.above === undefined ? "" : String(v.above);
    r.below.value = v.below === undefined ? "" : String(v.below);
    r.equals.value = v.equals === undefined ? "" : (typeof v.equals === "string" ? v.equals : JSON.stringify(v.equals));
    r.changed.checked = !!w.changed;
    r.action.value = t.action || "start";
    r.enabled.checked = d.enabled !== false;
    r.note.value = d.note || "";
    r.error.textContent = "";
    rEditor.classList.remove("d-none");
    r.name.focus();
    rEditor.scrollIntoView({ block: "nearest" });
  }

  // "92" is the number 92, true and false are yes and no, anything else
  // is text.
  function equalsValue(text) {
    if (text === "true" || text === "false") { return text === "true"; }
    var n = Number(text);
    return isFinite(n) ? n : text;
  }

  function closeRuleEditor() {
    rEditor.classList.add("d-none");
    editingRule = null;
  }

  rEditor.addEventListener("submit", function (e) {
    e.preventDefault();
    var w = { schedule: r.source.value, ended: r.ended.value };
    if (r.label.value.trim() !== "") { w.label = r.label.value.trim(); }
    var times = Number(r.times.value || 1);
    if (times !== 1) { w.times = times; }
    var field = r.field.value.trim();
    if (field !== "") {
      var test = { field: field };
      var bad = null;
      [["above", r.above], ["below", r.below]].forEach(function (pair) {
        var text = pair[1].value.trim();
        if (text === "") { return; }
        var n = Number(text);
        if (!isFinite(n)) { bad = pair[0] + " must be a number"; return; }
        test[pair[0]] = n;
      });
      var eq = r.equals.value.trim();
      if (eq !== "") { test.equals = equalsValue(eq); }
      if (bad) { r.error.textContent = "Not saved: " + bad + "."; return; }
      w.value = test;
    }
    if (r.changed.checked) { w.changed = true; }
    var d = { name: r.name.value.trim(), when: w, then: { action: r.action.value, schedule: r.target.value }, enabled: r.enabled.checked };
    if (r.note.value.trim() !== "") { d.note = r.note.value.trim(); }
    invoke("rules_save", { oldName: editingRule, definition: d }).then(function (r2) {
      reply = r2;
      closeRuleEditor();
      render();
      setStatus(ruleStatus, "Saved " + d.name + ".");
    }, function (err) {
      r.error.textContent = "Not saved: " + err + ".";
    });
  });

  $("btn-rule-cancel").addEventListener("click", closeRuleEditor);
  $("btn-rule-add").addEventListener("click", function () { openRuleEditor(null); });

  ruleRows.addEventListener("click", function (e) {
    var b = e.target.closest("button[data-action]");
    if (!b) { return; }
    var name = b.closest("tr").dataset.name;
    if (b.dataset.action === "edit") {
      openRuleEditor(name);
    } else if (b.dataset.action === "delete") {
      if (!confirmed(b)) { setStatus(ruleStatus, "Click Delete again to delete " + name + "."); return; }
      invoke("rules_delete", { name: name }).then(function () {
        setStatus(ruleStatus, "Deleted " + name + ".");
      }, function (err) {
        setStatus(ruleStatus, name + ": " + err + ".");
      });
    }
  });

  // Listening starts before the first load, so no change is missed.
  load();
  tab.addEventListener("shown.bs.tab", load);
})();
