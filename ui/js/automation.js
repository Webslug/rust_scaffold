/* Jobs tab: Triggers, Schedules and Actions (src-tauri/src/automation.rs,
   jobs.json in the settings folder).

   Each section has a list and an editor. Saving sends the whole entry to
   Rust, which checks it against the rest of jobs.json and refuses it, with
   nothing written, when it is not valid; the reason is shown under the
   editor. Rust pushes the whole list after every change as an
   "automation:changed" window event (a trigger check, a schedule run, an
   action finishing), so the lists follow without polling.

   "Then start" lists the Actions a Trigger, Schedule or Action starts.
   "New action" there opens the Action editor inside the editor being
   filled in; the saved Action is added to its list. */
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
  var WORDS = { trigger: "trigger", schedule: "schedule", action: "action" };

  var tab = document.getElementById("tab-jobs");
  var status = document.getElementById("auto-status");
  var skippedBox = document.getElementById("auto-skipped");
  var reply = { triggers: [], schedules: [], actions: [], skipped: [] };
  var confirming = null;   // "section/name" of a Delete waiting for its second click
  var confirmTimer = null;

  function $(id) { return document.getElementById(id); }

  function el(tag, className, text) {
    var e = document.createElement(tag);
    if (className) { e.className = className; }
    if (text !== undefined && text !== null) { e.textContent = text; }
    return e;
  }

  function say(text) {
    status.textContent = text;
  }

  function errorText(err) {
    return String(err && err.message ? err.message : err);
  }

  // 12:41:03 today, else 30 Jan 12:41.
  function when(ms) {
    if (!ms) { return "–"; }
    var d = new Date(ms);
    var today = new Date();
    if (d.toDateString() === today.toDateString()) { return d.toLocaleTimeString("en-GB"); }
    return d.toLocaleDateString("en-GB", { day: "numeric", month: "short" }) + " " +
      d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
  }

  function plural(n, word) {
    return n === 1 ? "1 " + word : n + " " + word + "s";
  }

  function everyWords(seconds) {
    if (seconds % 86400 === 0) { return plural(seconds / 86400, "day"); }
    if (seconds % 3600 === 0) { return plural(seconds / 3600, "hour"); }
    if (seconds % 60 === 0) { return plural(seconds / 60, "minute"); }
    return plural(seconds, "second");
  }

  function listOf(section) {
    return section === "trigger" ? reply.triggers : section === "schedule" ? reply.schedules : reply.actions;
  }

  function names(section) {
    return listOf(section).map(function (x) { return x.name; });
  }
  window.JobForms.names = names;

  // ---- Sections ---------------------------------------------------------------------

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

  // ---- "Then start": the Actions an entry starts -----------------------------------------

  function thenList(container, prefix, label, canCreate, emptyText) {
    var chosen = [];
    var head = el("span", "form-label", label);
    var chips = el("div", "jobs-chips");
    var add = el("select", "form-select form-select-sm jobs-then-select");
    add.id = prefix + "then-add";
    add.setAttribute("aria-label", label + ": add an action");
    var row = el("div", "jobs-then-add");
    row.appendChild(add);
    var slot = el("div", "jobs-then-slot");
    var newButton = null;
    if (canCreate) {
      newButton = el("button", "btn btn-secondary btn-sm");
      newButton.type = "button";
      newButton.id = prefix + "then-new";
      newButton.appendChild(el("i", "bi bi-plus-lg me-1"));
      newButton.appendChild(document.createTextNode("New action"));
      row.appendChild(newButton);
    }
    container.textContent = "";
    container.appendChild(head);
    container.appendChild(chips);
    container.appendChild(row);
    container.appendChild(slot);

    function draw() {
      chips.textContent = "";
      if (!chosen.length) {
        chips.appendChild(el("span", "jobs-subtle", emptyText));
      }
      chosen.forEach(function (name) {
        var chip = el("span", "jobs-chip");
        chip.appendChild(el("span", "", name));
        if (names("action").indexOf(name) < 0) { chip.appendChild(el("span", "jobs-subtle", " (missing)")); }
        var x = el("button", "btn btn-sm jobs-chip-remove");
        x.type = "button";
        x.setAttribute("aria-label", "Remove " + name);
        x.title = "Remove";
        x.appendChild(el("i", "bi bi-x-lg"));
        x.addEventListener("click", function () {
          chosen = chosen.filter(function (n) { return n !== name; });
          draw();
        });
        chip.appendChild(x);
        chips.appendChild(chip);
      });
      add.textContent = "";
      var first = el("option", "", "Add an action...");
      first.value = "";
      add.appendChild(first);
      names("action").filter(function (n) { return chosen.indexOf(n) < 0 && n !== excluded; }).forEach(function (n) {
        var o = el("option", "", n);
        o.value = n;
        add.appendChild(o);
      });
    }

    var excluded = null; // an Action cannot start itself
    add.addEventListener("change", function () {
      if (add.value) { chosen.push(add.value); }
      draw();
    });

    var api = {
      slot: slot,
      newButton: newButton,
      set: function (list, self) {
        chosen = (list || []).slice();
        excluded = self || null;
        draw();
      },
      get: function () { return chosen.slice(); },
      add: function (name) {
        if (chosen.indexOf(name) < 0) { chosen.push(name); }
        draw();
      },
      redraw: draw
    };
    return api;
  }

  // ---- Editors ------------------------------------------------------------------------

  function showEditor(card, title, text) {
    title.textContent = text;
    card.classList.remove("d-none");
    var first = card.querySelector("input[type=text]");
    card.scrollIntoView({ block: "nearest" });
    if (first) { first.focus(); }
  }

  function save(section, oldName, definition, card, errorBox) {
    errorBox.textContent = "";
    return invoke("automation_save", { section: section, oldName: oldName, definition: definition }).then(function (r) {
      apply(r);
      card.classList.add("d-none");
      say("Saved " + WORDS[section] + " \"" + definition.name + "\".");
      return true;
    }, function (err) {
      errorBox.textContent = "Not saved: " + errorText(err) + ".";
      return false;
    });
  }

  // ---- Trigger editor ----

  var trig = {
    card: $("trigger-editor"),
    title: $("trigger-editor-title"),
    error: $("trigger-error"),
    test: $("trigger-test-result"),
    oldName: null,
    then: thenList($("trigger-then"), "trigger-", "Then start", true, "No actions yet: choose one below or make a new one.")
  };

  function syncTriggerWhen() {
    var w = $("trigger-when").value;
    $("trigger-value-box").classList.toggle("d-none", w === "changes");
    if ((w === "above" || w === "below") && $("trigger-watch-type").value === "web_watch") { $("trigger-read").value = "number"; }
  }
  $("trigger-when").addEventListener("change", syncTriggerWhen);

  // Shows the fields of the chosen watch (data-watch lists the types).
  function syncWatchType() {
    var type = $("trigger-watch-type").value;
    trig.card.querySelectorAll("[data-watch]").forEach(function (box) {
      box.classList.toggle("d-none", box.dataset.watch.split(" ").indexOf(type) < 0);
    });
    // A file has no number: only "changes" and "equals" make sense.
    ["above", "below"].forEach(function (v) {
      $("trigger-when").querySelector('option[value="' + v + '"]').disabled = type === "file";
    });
    if (type === "file" && ($("trigger-when").value === "above" || $("trigger-when").value === "below")) {
      $("trigger-when").value = "changes";
      syncTriggerWhen();
    }
  }
  $("trigger-watch-type").addEventListener("change", syncWatchType);

  function setUnit(seconds, numberBox, unitBox) {
    var units = [86400, 3600, 60, 1];
    var unit = units.filter(function (u) { return seconds % u === 0; })[0] || 1;
    unitBox.value = String(unit);
    numberBox.value = String(seconds / unit);
  }

  function openTrigger(view) {
    var d = view ? view.definition : null;
    var w = d ? d.watch : { type: "web_watch" };
    trig.oldName = view ? view.name : null;
    $("trigger-name").value = d ? d.name : "";
    $("trigger-watch-type").value = w.type;
    var web = w.type === "web_watch";
    $("trigger-url").value = web ? w.url || "" : "";
    $("trigger-find").value = web ? w.find || "id" : "id";
    $("trigger-element").value = web ? w.name || "" : "";
    $("trigger-read").value = web ? w.read_as || "text" : "text";
    $("trigger-disk-path").value = w.type === "disk" ? w.path : "/";
    $("trigger-disk-measure").value = w.type === "disk" && w.measure ? w.measure : "used_percent";
    $("trigger-file-path").value = w.type === "file" ? w.path : "";
    $("trigger-file-for").value = w.type === "file" && w.watch_for ? w.watch_for : "contents";
    setUnit(d ? d.every_seconds : 300, $("trigger-every"), $("trigger-every-unit"));
    var c = d ? d.when : { is: "changes" };
    $("trigger-when").value = c.is;
    $("trigger-value").value = c.value === undefined ? "" : String(c.value);
    $("trigger-max").value = d && d.max_fires ? String(d.max_fires) : "";
    $("trigger-enabled").checked = d ? d.enabled !== false : true;
    syncToggle("trigger-enabled");
    trig.then.set(d ? d.then : []);
    trig.error.textContent = "";
    trig.test.textContent = "";
    syncWatchType();
    syncTriggerWhen();
    showView("trigger");
    showEditor(trig.card, trig.title, view ? "Edit trigger \"" + view.name + "\"" : "New trigger");
  }

  // "£5,000" or "5000" as a number.
  function number(text) {
    var n = Number(String(text).replace(/[£$€¥,\s]/g, ""));
    return String(text).trim() === "" || !isFinite(n) ? null : n;
  }

  // The watch as jobs.json holds it.
  function readWatch() {
    var type = $("trigger-watch-type").value;
    if (type === "web_watch") {
      var find = $("trigger-find").value;
      var element = $("trigger-element").value.trim().replace(find === "id" ? /^#/ : /^\./, "");
      return { type: type, url: $("trigger-url").value.trim(), find: find, name: element, read_as: $("trigger-read").value };
    }
    if (type === "disk") { return { type: type, path: $("trigger-disk-path").value.trim(), measure: $("trigger-disk-measure").value }; }
    if (type === "file") { return { type: type, path: $("trigger-file-path").value.trim(), watch_for: $("trigger-file-for").value }; }
    return { type: type };
  }

  function readTrigger() {
    var watch = readWatch();
    var numbers = watch.type === "web_watch" ? watch.read_as === "number" : watch.type !== "file";
    var every = Number($("trigger-every").value) * Number($("trigger-every-unit").value);
    if (!isFinite(every) || every < 1) { throw new Error("check every needs a number"); }
    var is = $("trigger-when").value;
    var c = { is: is };
    var raw = $("trigger-value").value;
    if (is === "above" || is === "below") {
      c.value = number(raw);
      if (c.value === null) { throw new Error("\"" + (is === "above" ? "is above" : "is below") + "\" needs a number, for example 5000"); }
    } else if (is === "equals") {
      if (raw.trim() === "") { throw new Error("\"equals\" needs a value"); }
      c.value = numbers && number(raw) !== null ? number(raw) : raw.trim();
    }
    var d = {
      name: $("trigger-name").value.trim(),
      enabled: $("trigger-enabled").checked,
      watch: watch,
      every_seconds: every,
      when: c,
      then: trig.then.get()
    };
    var max = $("trigger-max").value.trim();
    if (max) { d.max_fires = Number(max); }
    return d;
  }

  $("btn-trigger-new").addEventListener("click", function () { openTrigger(null); });
  $("btn-trigger-cancel").addEventListener("click", function () { trig.card.classList.add("d-none"); closeActionEditor(); });
  $("btn-trigger-save").addEventListener("click", function () {
    var d;
    try { d = readTrigger(); } catch (e) { trig.error.textContent = "Not saved: " + e.message + "."; return; }
    if (!d.name) { trig.error.textContent = "Not saved: the trigger needs a name."; return; }
    save("trigger", trig.oldName, d, trig.card, trig.error).then(function (ok) { if (ok) { closeActionEditor(); } });
  });

  $("btn-trigger-test").addEventListener("click", function () {
    var d;
    try { d = readTrigger(); } catch (e) { trig.test.textContent = e.message + "."; return; }
    trig.test.textContent = "Reading the page...";
    invoke("automation_test_trigger", { name: trig.oldName, definition: d }).then(function (t) {
      if (t.error) {
        trig.test.textContent = "Could not read it: " + t.error;
        return;
      }
      var v = t.value;
      var text = "Read \"" + v.text + "\"" + (v.number !== undefined ? " (the number " + v.number + ")" : "") + " just now. ";
      if (t.holds !== null) {
        text += t.holds ? "The condition is true now." : "The condition is not true now.";
      } else if (t.changed !== null) {
        text += t.changed ? "That differs from the last reading." : "That is the same as the last reading.";
      } else {
        text += "After saving, it fires when the value differs from the reading before.";
      }
      trig.test.textContent = text;
    }, function (err) {
      trig.test.textContent = errorText(err) + ".";
    });
  });

  // ---- Schedule editor ----

  var sched = {
    card: $("schedule-editor"),
    title: $("schedule-editor-title"),
    error: $("schedule-error"),
    oldName: null,
    then: thenList($("schedule-then"), "schedule-", "Then start", true, "No actions yet: choose one below or make a new one.")
  };

  function syncScheduleWhen() {
    var w = $("schedule-when").value;
    sched.card.querySelectorAll("[data-when]").forEach(function (box) {
      box.classList.toggle("d-none", box.dataset.when.split(" ").indexOf(w) < 0);
    });
  }
  $("schedule-when").addEventListener("change", syncScheduleWhen);

  function openSchedule(view) {
    var d = view ? view.definition : null;
    var w = d ? d.when : { type: "daily", time: "08:00" };
    sched.oldName = view ? view.name : null;
    $("schedule-name").value = d ? d.name : "";
    $("schedule-when").value = w.type;
    $("schedule-at").value = w.at || "";
    var m = /^(\d+)\s*([smhd])$/.exec(w.interval || "");
    $("schedule-every").value = m ? m[1] : "1";
    $("schedule-every-unit").value = m && m[2] !== "s" ? m[2] : "h";
    $("schedule-weekday").value = w.day && typeof w.day === "string" ? w.day : "monday";
    $("schedule-monthday").value = typeof w.day === "number" ? String(w.day) : "1";
    $("schedule-time").value = w.time || "08:00";
    $("schedule-max").value = d && d.max_runs ? String(d.max_runs) : "";
    $("schedule-enabled").checked = d ? d.enabled !== false : true;
    syncToggle("schedule-enabled");
    sched.then.set(d ? d.then : []);
    sched.error.textContent = "";
    syncScheduleWhen();
    showView("schedule");
    showEditor(sched.card, sched.title, view ? "Edit schedule \"" + view.name + "\"" : "New schedule");
  }

  function readSchedule() {
    var type = $("schedule-when").value;
    var w = { type: type };
    if (type === "once") { w.at = $("schedule-at").value.trim(); }
    if (type === "every") { w.interval = $("schedule-every").value.trim() + $("schedule-every-unit").value; }
    if (type === "weekly") { w.day = $("schedule-weekday").value; }
    if (type === "monthly") { w.day = Number($("schedule-monthday").value); }
    if (type === "daily" || type === "weekly" || type === "monthly") { w.time = $("schedule-time").value.trim(); }
    var d = { name: $("schedule-name").value.trim(), enabled: $("schedule-enabled").checked, when: w, then: sched.then.get() };
    var max = $("schedule-max").value.trim();
    if (max) { d.max_runs = Number(max); }
    return d;
  }

  $("btn-schedule-new").addEventListener("click", function () { openSchedule(null); });
  $("btn-schedule-cancel").addEventListener("click", function () { sched.card.classList.add("d-none"); closeActionEditor(); });
  $("btn-schedule-save").addEventListener("click", function () {
    var d = readSchedule();
    if (!d.name) { sched.error.textContent = "Not saved: the schedule needs a name."; return; }
    save("schedule", sched.oldName, d, sched.card, sched.error).then(function (ok) { if (ok) { closeActionEditor(); } });
  });

  // ---- Action editor ----

  var act = {
    card: $("action-editor"),
    title: $("action-editor-title"),
    error: $("action-error"),
    home: $("action-home"),
    oldName: null,
    host: null, // the "Then start" list it was opened from, if any
    fields: window.JobForms.mount($("action-kind-fields"), "action-"),
    then: thenList($("action-then"), "action-", "After it succeeds, start", false, "Nothing. To chain actions, choose one below.")
  };

  function closeActionEditor() {
    act.card.classList.add("d-none");
    act.home.appendChild(act.card);
    act.host = null;
  }

  function openAction(view, host) {
    var d = view ? view.definition : null;
    act.oldName = view ? view.name : null;
    act.host = host || null;
    (host ? host.slot : act.home).appendChild(act.card);
    $("action-name").value = d ? d.name : "";
    $("action-enabled").checked = d ? d.enabled !== false : true;
    syncToggle("action-enabled");
    act.fields.set(d ? d.kind : null, d ? d.input : undefined);
    act.then.set(d ? d.then : [], d ? d.name : null);
    act.error.textContent = "";
    if (!host) { showView("action"); }
    showEditor(act.card, act.title, view ? "Edit action \"" + view.name + "\"" : "New action");
  }

  $("btn-action-new").addEventListener("click", function () { openAction(null, null); });
  [trig.then, sched.then].forEach(function (list) {
    list.newButton.addEventListener("click", function () { openAction(null, list); });
  });
  $("btn-action-cancel").addEventListener("click", closeActionEditor);
  $("btn-action-save").addEventListener("click", function () {
    var job;
    try { job = act.fields.read(); } catch (e) { act.error.textContent = "Not saved: " + e.message + "."; return; }
    var d = { name: $("action-name").value.trim(), enabled: $("action-enabled").checked, kind: job.kind, input: job.input, then: act.then.get() };
    if (!d.name) { act.error.textContent = "Not saved: the action needs a name."; return; }
    var host = act.host;
    save("action", act.oldName, d, act.card, act.error).then(function (ok) {
      if (!ok) { return; }
      closeActionEditor();
      if (host) {
        host.add(d.name);
        // Back in the trigger or schedule editor: Save is the next step.
        var next = host.slot.closest(".jobs-editor").querySelector(".jobs-actions .btn-primary");
        if (next) { next.focus(); }
      }
    });
  });

  // Enter in a one-line box saves the editor it is in (the innermost, so
  // Enter in the action editor opened inside a trigger saves the action).
  [trig.card, sched.card, act.card].forEach(function (card) {
    card.addEventListener("keydown", function (e) {
      if (e.key !== "Enter" || e.target.tagName !== "INPUT" || e.target.type === "checkbox") { return; }
      if (e.target.closest(".jobs-editor") !== card) { return; }
      e.preventDefault();
      card.querySelector(".jobs-actions .btn-primary").click();
    });
  });

  // ---- Lists --------------------------------------------------------------------------

  function button(icon, label, action, text) {
    var b = el("button", "btn btn-secondary btn-sm");
    b.type = "button";
    b.dataset.action = action;
    b.setAttribute("aria-label", label);
    b.title = label;
    b.appendChild(el("i", "bi " + icon + (text ? " me-1" : "")));
    if (text) { b.appendChild(document.createTextNode(text)); }
    return b;
  }

  function onSwitch(section, item) {
    var box = el("div", "form-check form-switch mb-0");
    var input = el("input", "form-check-input");
    input.type = "checkbox";
    input.checked = item.enabled;
    input.dataset.action = "enabled";
    input.setAttribute("aria-label", WORDS[section] + " \"" + item.name + "\" on");
    box.appendChild(input);
    return box;
  }

  function nameCell(td, name, lines) {
    td.appendChild(el("div", "jobs-name", name));
    lines.filter(Boolean).forEach(function (line) { td.appendChild(el("div", "jobs-subtle", line)); });
  }

  function thenText(list) {
    return list.length ? "then: " + list.join(", ") : "starts no actions yet";
  }

  function buttons(td, section, item, extra) {
    var row = el("div", "jobs-button-row");
    (extra || []).forEach(function (b) { row.appendChild(b); });
    row.appendChild(button("bi-pencil", "Edit " + item.name, "edit", "Edit"));
    row.appendChild(button("bi-copy", "Duplicate " + item.name, "duplicate"));
    var del = button("bi-trash", "Delete " + item.name, "delete");
    // Waiting for the second click: red, same size and place, so the
    // button does not move away from the pointer.
    if (confirming === section + "/" + item.name) {
      del.classList.add("jobs-confirm");
      del.setAttribute("aria-label", "Press again to delete " + item.name);
      del.title = "Press again to delete";
    }
    row.appendChild(del);
    td.appendChild(row);
  }

  function emptyRow(tbody, cols, text) {
    var tr = el("tr");
    var td = el("td", "jobs-empty", text);
    td.colSpan = cols;
    tr.appendChild(td);
    tbody.appendChild(tr);
  }

  function row(tbody, section, item) {
    var tr = el("tr");
    tr.dataset.section = section;
    tr.dataset.name = item.name;
    tbody.appendChild(tr);
    var on = el("td");
    on.appendChild(onSwitch(section, item));
    tr.appendChild(on);
    return tr;
  }

  function td(tr, className) {
    var c = el("td", className);
    tr.appendChild(c);
    return c;
  }

  function renderTriggers() {
    var tbody = $("trigger-rows");
    tbody.textContent = "";
    if (!reply.triggers.length) { emptyRow(tbody, 4, "No triggers yet. Press New trigger."); }
    reply.triggers.forEach(function (t) {
      var tr = row(tbody, "trigger", t);
      nameCell(td(tr, "jobs-wrap"), t.name, [
        t.watching,
        "when it " + t.when + ", checked every " + everyWords(t.every_seconds),
        thenText(t.then)
      ]);
      var v = td(tr, "jobs-wrap");
      if (t.checking) {
        v.appendChild(el("span", "badge badge-info", "Checking"));
      } else if (t.last_error) {
        var bad = el("span", "badge badge-danger", "Could not read");
        bad.title = t.last_error;
        v.appendChild(bad);
        var why = el("div", "jobs-subtle jobs-result-line", t.last_error);
        why.title = t.last_error;
        v.appendChild(why);
      }
      if (t.last_value) {
        v.appendChild(el("div", "jobs-value", t.last_value.text === "" ? "(empty)" : t.last_value.text));
      } else if (!t.checking && !t.last_error) {
        v.appendChild(el("span", "jobs-subtle", t.enabled ? "not read yet" : "off"));
      }
      if (t.last_checked_ms) { v.appendChild(el("div", "jobs-subtle", "checked " + when(t.last_checked_ms))); }
      var fired = t.fires ? "fired " + plural(t.fires, "time") + (t.max_fires ? " of " + t.max_fires : "") : (t.max_fires ? "fires at most " + plural(t.max_fires, "time") : "");
      if (fired) { v.appendChild(el("div", "jobs-subtle", fired)); }
      buttons(td(tr, "jobs-buttons"), "trigger", t, [button("bi-lightning-fill", "Fire " + t.name + " now: read the page and start its actions", "fire", "Fire now")]);
    });
  }

  function renderSchedules() {
    var tbody = $("schedule-rows");
    tbody.textContent = "";
    if (!reply.schedules.length) { emptyRow(tbody, 4, "No schedules yet. Press New schedule."); }
    reply.schedules.forEach(function (s) {
      var tr = row(tbody, "schedule", s);
      nameCell(td(tr, "jobs-wrap"), s.name, [s.when, thenText(s.then)]);
      var next = td(tr, "jobs-wrap");
      next.appendChild(el("div", "", !s.enabled ? "off" : s.next_ms ? when(s.next_ms) : "never again"));
      var runs = s.max_runs ? "ran " + s.runs + " of " + s.max_runs : "ran " + plural(s.runs, "time");
      next.appendChild(el("div", "jobs-subtle", runs + (s.last_run_ms ? ", last " + when(s.last_run_ms) : "")));
      buttons(td(tr, "jobs-buttons"), "schedule", s, [button("bi-play-fill", "Run " + s.name + " now: start its actions", "run-schedule", "Run now")]);
    });
  }

  function renderActions() {
    var tbody = $("action-rows");
    tbody.textContent = "";
    if (!reply.actions.length) { emptyRow(tbody, 4, "No actions yet. Press New action."); }
    reply.actions.forEach(function (a) {
      var tr = row(tbody, "action", a);
      nameCell(td(tr, "jobs-wrap"), a.name, [a.summary, a.then.length ? "then: " + a.then.join(", ") : ""]);
      var r = td(tr, "jobs-wrap");
      var last = a.history.length ? a.history[a.history.length - 1] : null;
      if (a.running.length) {
        r.appendChild(el("span", "badge badge-info", a.running.length === 1 ? "Running" : a.running.length + " running"));
      } else if (last) {
        var st = STATES[last.state] || STATES.failed;
        r.appendChild(el("span", "badge " + st.badge, st.label));
        r.appendChild(el("span", "jobs-subtle", " " + when(last.at_ms) + ", by " + last.by));
        var detail = last.state === "done" ? (last.output || last.message) : (last.message || last.output);
        if (detail) {
          var line = el("div", "jobs-subtle jobs-result-line", detail.trim().split("\n").pop());
          line.title = detail;
          r.appendChild(line);
        }
      } else {
        r.appendChild(el("span", "jobs-subtle", "not run yet"));
      }
      var extra = [button("bi-play-fill", "Run " + a.name + " now", "run", "Run")];
      var job = a.running.length ? a.running[a.running.length - 1] : (last && last.job);
      if (job) { extra.push(button("bi-search", "Details of " + a.name + ", job #" + job, "inspect")); }
      buttons(td(tr, "jobs-buttons"), "action", a, extra);
      tr.dataset.job = job || "";
    });
  }

  // The editors' Enabled / Disabled toggles say which they are.
  function syncToggle(id) {
    var box = $(id);
    var label = document.querySelector('label[for="' + id + '"]');
    if (label) { label.textContent = box.checked ? "Enabled" : "Disabled"; }
  }
  ["trigger-enabled", "schedule-enabled", "action-enabled"].forEach(function (id) {
    $(id).addEventListener("change", function () { syncToggle(id); });
  });

  var listeners = []; // the Overview (js/jobs_overview.js) redraws with each reply

  function apply(r) {
    reply = r;
    listeners.forEach(function (fn) { fn(r); });
    $("auto-path").textContent = r.path;
    $("auto-max-chain").textContent = String(r.max_chain);
    skippedBox.classList.toggle("d-none", !r.skipped.length);
    skippedBox.textContent = r.skipped.length
      ? "Not loaded from jobs.json (fix them on the page or in the file, then Reload): " + r.skipped.join("; ")
      : "";
    renderTriggers();
    renderSchedules();
    renderActions();
    trig.then.redraw();
    sched.then.redraw();
    act.then.redraw();
  }

  function load() {
    return invoke("automation_list").then(apply, function (err) { say("Could not load the jobs: " + errorText(err)); });
  }

  // ---- Row buttons ----------------------------------------------------------------------

  function find(section, name) {
    return listOf(section).filter(function (x) { return x.name === name; })[0];
  }

  function rowAction(e) {
    var target = e.target.closest("[data-action]");
    if (!target) { return; }
    var tr = target.closest("tr");
    var section = tr.dataset.section;
    var name = tr.dataset.name;
    var item = find(section, name);
    if (!item) { return; }
    var what = WORDS[section] + " \"" + name + "\"";
    switch (target.dataset.action) {
      case "enabled":
        invoke("automation_set_enabled", { section: section, name: name, enabled: target.checked }).then(function (r) {
          apply(r);
          say("Switched " + what + (target.checked ? " on." : " off."));
        }, function (err) {
          target.checked = !target.checked;
          say("Could not switch " + what + ": " + errorText(err) + ".");
        });
        break;
      case "edit":
        if (section === "trigger") { openTrigger(item); }
        if (section === "schedule") { openSchedule(item); }
        if (section === "action") { openAction(item, null); }
        break;
      case "duplicate":
        invoke("automation_duplicate", { section: section, name: name }).then(function (copy) {
          say("Copied " + what + " as \"" + copy + "\"" + (section === "action" ? "." : ", switched off."));
          load();
        }, function (err) { say("Could not copy " + what + ": " + errorText(err) + "."); });
        break;
      case "delete":
        if (confirming !== section + "/" + name) {
          // Only this button changes (red, same place); a list redraw would
          // replace the button under the pointer.
          var before = document.querySelector("button.jobs-confirm");
          if (before) { before.classList.remove("jobs-confirm"); }
          confirming = section + "/" + name;
          target.classList.add("jobs-confirm");
          target.setAttribute("aria-label", "Press again to delete " + name);
          clearTimeout(confirmTimer);
          confirmTimer = setTimeout(function () { confirming = null; apply(reply); }, 4000);
          say("Press Delete again to delete " + what + ".");
          return;
        }
        confirming = null;
        invoke("automation_delete", { section: section, name: name }).then(function (r) {
          apply(r);
          say("Deleted " + what + ".");
        }, function (err) {
          apply(reply);
          say("Not deleted: " + errorText(err) + ".");
        });
        break;
      case "run":
        invoke("automation_run", { name: name }).then(function (id) {
          say(id ? "Started " + what + " (job #" + id + ")." : "Ran " + what + ".");
        }, function (err) { say("Could not run " + what + ": " + errorText(err) + "."); });
        break;
      case "fire":
        target.disabled = true;
        say("Reading the page for " + what + "...");
        invoke("automation_fire_trigger", { name: name }).then(function (value) {
          var started = item.then.length ? "started " + item.then.join(", ") : "it starts no actions yet";
          say("Fired " + what + ": read \"" + value.text + "\", " + started + ".");
        }, function (err) {
          say("Could not fire " + what + ": " + errorText(err) + ".");
        }).then(function () { target.disabled = false; });
        break;
      case "run-schedule":
        invoke("automation_run_schedule", { name: name }).then(function () {
          say("Ran " + what + " now: started " + item.then.join(", ") + ".");
        }, function (err) { say("Could not run " + what + ": " + errorText(err) + "."); });
        break;
      case "inspect":
        window.JobsInspect.open(Number(tr.dataset.job));
        break;
    }
  }
  ["trigger-rows", "schedule-rows", "action-rows"].forEach(function (id) {
    $(id).addEventListener("click", function (e) {
      if (e.target.matches("input[type=checkbox]")) { return; } // handled on change
      rowAction(e);
    });
    $(id).addEventListener("change", rowAction);
  });

  $("btn-auto-reload").addEventListener("click", function () {
    invoke("automation_reload").then(function (r) { apply(r); say("Read jobs.json again."); });
  });

  tauri.event.listen("automation:changed", function (e) { apply(e.payload); });

  load();
  tab.addEventListener("shown.bs.tab", function () { say(""); load(); });

  // Opens an entry's editor (the Overview's rows).
  function openEntry(section, name) {
    var item = find(section, name);
    if (!item) { return; }
    if (section === "trigger") { openTrigger(item); }
    if (section === "schedule") { openSchedule(item); }
    if (section === "action") { openAction(item, null); }
  }

  window.AutomationPage = {
    say: say,
    apply: apply,
    open: openEntry,
    showView: showView,
    onReply: function (fn) { listeners.push(fn); fn(reply); },
    when: when,
    plural: plural,
    everyWords: everyWords,
    states: STATES
  };
})();
