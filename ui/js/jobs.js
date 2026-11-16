/* Jobs tab: the "Running now" strip and the job details card. Every job is
   started by an Action (js/automation.js, src-tauri/src/automation.rs) and
   run by the Rust job queue (src-tauri/src/jobs.rs).

   Rust pushes every change to a job as a "jobs:changed" window event, so the
   strip follows each job without polling. A queued or running job is shown
   with Inspect and Kill; finished jobs leave the strip (their results are
   listed with their Action). window.JobsInspect.open(id) shows a job's
   details, also after it has finished, while the queue still lists it (the
   newest 50 finished jobs). */
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

  // What a kind touches outside the app (Capability in jobs.rs).
  var CAPABILITIES = {
    network: "network",
    gpu: "graphics card",
    write_files: "writes files",
    run_programs: "runs programs",
    costs_money: "costs money"
  };

  var tab = document.getElementById("tab-jobs");
  var strip = document.getElementById("jobs-running");
  var rows = document.getElementById("jobs-running-rows");
  var inspectCard = document.getElementById("jobs-inspect");
  var inspectTitle = document.getElementById("jobs-inspect-title");
  var inspectList = document.getElementById("jobs-inspect-list");
  var outputBox = document.getElementById("jobs-output-box");
  var outputPre = document.getElementById("jobs-output");
  var outputTitle = document.getElementById("jobs-output-title");

  var jobs = {};       // id -> job, as Rust sends it
  var selected = null; // id shown in the details card
  var timer = null;    // updates the running times once a second

  function isFinished(job) {
    return job.state !== "queued" && job.state !== "running";
  }

  function duration(ms) {
    if (ms === null || ms === undefined) { return "–"; }
    if (ms < 1000) { return ms + " ms"; }
    if (ms < 60000) { return (ms / 1000).toFixed(1) + " s"; }
    var minutes = Math.floor(ms / 60000);
    if (minutes < 60) { return minutes + " min " + Math.floor((ms % 60000) / 1000) + " s"; }
    return Math.floor(minutes / 60) + " h " + (minutes % 60) + " min";
  }

  function clock(unixMs) {
    return unixMs ? new Date(unixMs).toLocaleTimeString("en-GB") : "–";
  }

  function uses(job) {
    if (!job.capabilities.length) { return "nothing outside the app"; }
    return job.capabilities.map(function (c) { return CAPABILITIES[c] || c; }).join(", ");
  }

  // The Action a job belongs to, and what started it.
  function actionOf(job) {
    return job.origin && job.origin.action ? job.origin.action : window.JobForms.label(job.kind);
  }

  function startedBy(origin) {
    if (!origin) { return "–"; }
    switch (origin.by) {
      case "run": return "you (Run)";
      case "trigger": return "trigger " + origin.trigger;
      case "schedule": return "schedule " + origin.schedule;
      case "action": return "action " + origin.after;
      default: return origin.by;
    }
  }

  function json(value) {
    return value === null || value === undefined ? "–" : JSON.stringify(value);
  }

  function button(icon, label, action, text) {
    var b = document.createElement("button");
    b.type = "button";
    b.className = "btn btn-secondary btn-sm";
    b.dataset.action = action;
    b.setAttribute("aria-label", label);
    b.title = label;
    var i = document.createElement("i");
    i.className = "bi " + icon + (text ? " me-1" : "");
    b.appendChild(i);
    if (text) { b.appendChild(document.createTextNode(text)); }
    return b;
  }

  // ---- Running now --------------------------------------------------------------

  function cell(tr, text, className) {
    var td = document.createElement("td");
    if (className) { td.className = className; }
    if (text !== undefined) { td.textContent = text; }
    tr.appendChild(td);
    return td;
  }

  function render() {
    var active = Object.keys(jobs).map(Number).filter(function (id) { return !isFinished(jobs[id]); }).sort(function (a, b) { return a - b; });
    rows.textContent = "";
    active.forEach(function (id) {
      var job = jobs[id];
      var state = STATES[job.state] || STATES.failed;
      var tr = document.createElement("tr");
      tr.dataset.id = id;
      tr.classList.toggle("jobs-selected", selected === id);
      cell(tr, actionOf(job), "jobs-name");
      var badgeCell = cell(tr);
      var badge = document.createElement("span");
      badge.className = "badge " + state.badge;
      badge.textContent = state.label;
      badgeCell.appendChild(badge);
      cell(tr, startedBy(job.origin), "jobs-extra");
      var note = job.total > 0 ? job.done + " / " + job.total : (job.note || "");
      var noteCell = cell(tr, note, "jobs-note-cell");
      noteCell.title = note;
      var started = job.started_ms ? duration(Date.now() - job.started_ms) : "waiting";
      cell(tr, started, "jobs-number");
      var buttons = cell(tr, undefined, "jobs-buttons");
      buttons.appendChild(button("bi-search", "Inspect " + actionOf(job), "inspect"));
      buttons.appendChild(button("bi-x-octagon", "Kill " + actionOf(job), "kill", "Kill"));
      rows.appendChild(tr);
    });
    strip.classList.toggle("d-none", active.length === 0);
    if (active.length && timer === null) {
      timer = setInterval(render, 1000);
    } else if (!active.length && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  function load() {
    return invoke("jobs_list").then(function (reply) {
      // A reply can arrive after events that are newer than it. Every change
      // carries a rising seq from Rust: keep local copies newer than the
      // snapshot (including jobs it has not seen yet), take the rest from it.
      var merged = {};
      reply.jobs.forEach(function (job) { merged[job.id] = job; });
      Object.keys(jobs).forEach(function (id) {
        if (jobs[id].seq > reply.seq) { merged[id] = jobs[id]; }
      });
      jobs = merged;
      render();
      if (selected !== null) { inspect(selected); }
    }).catch(function () {});
  }

  // ---- Details card ---------------------------------------------------------------

  function inspect(id) {
    invoke("jobs_inspect", { id: id }).then(function (info) {
      var job = info.job;
      var state = STATES[job.state] || STATES.failed;
      var given = job.given;
      var items = [
        ["Action", actionOf(job)],
        ["State", state.label],
        ["Kind", window.JobForms.label(job.kind)],
        ["Started by", startedBy(job.origin)],
        ["Uses", uses(job)],
        ["Needs", job.resources.length ? job.resources.join(", ") : "–"],
        ["Input", json(job.input)],
        ["Value given", given ? json(given.value) : "–"],
        ["Value before", given ? json(given.previous) : "–"],
        ["Output given", given && given.output_bytes ? given.output_bytes + " bytes" : "–"],
        ["Live note", job.note || "–"],
        ["Time limit", duration(job.timeout_ms)],
        ["Queued at", clock(job.created_ms)],
        ["Started at", clock(job.started_ms)],
        ["Finished at", clock(job.finished_ms)],
        ["Waited", duration(job.wait_ms)],
        ["Ran", duration(job.run_ms)],
        ["Place in queue", info.queue_position === null ? "–" : String(info.queue_position)],
        ["Result", job.label || "–"],
        ["Result value", json(job.value)],
        ["Message", job.message || "–"]
      ];
      inspectList.textContent = "";
      items.forEach(function (item) {
        var dt = document.createElement("dt");
        dt.textContent = item[0];
        var dd = document.createElement("dd");
        dd.textContent = item[1];
        inspectList.appendChild(dt);
        inspectList.appendChild(dd);
      });
      outputBox.classList.toggle("d-none", !info.output);
      outputPre.textContent = info.output || "";
      outputTitle.textContent = info.output_cut ? "Output (the newest part; older output was dropped)" : "Output";
      inspectTitle.textContent = actionOf(job) + ", job #" + job.id;
      inspectCard.classList.remove("d-none");
    }, function () {
      closeInspect();
      if (window.AutomationPage) { window.AutomationPage.say("That job is no longer kept; its result is listed with its action."); }
    });
  }

  function open(id) {
    selected = id;
    render();
    inspect(id);
    inspectCard.scrollIntoView({ block: "nearest" });
  }

  function closeInspect() {
    selected = null;
    inspectCard.classList.add("d-none");
    render();
  }

  // ---- Events and buttons -------------------------------------------------------

  tauri.event.listen("jobs:changed", function (e) {
    var job = e.payload;
    var before = jobs[job.id];
    if (before && before.seq > job.seq) { return; } // already have newer
    jobs[job.id] = job;
    render();
    if (selected === job.id) { inspect(job.id); }
  });

  // The clipboard API, or the older copy command where it is missing (the
  // same as the Errors page).
  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var area = document.createElement("textarea");
      area.value = text;
      area.className = "visually-hidden";
      document.body.appendChild(area);
      area.select();
      var ok = document.execCommand("copy");
      area.remove();
      (ok ? resolve : reject)(new Error("copy was refused"));
    });
  }

  document.getElementById("btn-jobs-output-copy").addEventListener("click", function () {
    var say = window.AutomationPage ? window.AutomationPage.say : function () {};
    copyText(outputPre.textContent).then(function () { say("Output copied."); }, function () { say("Could not copy the output."); });
  });

  rows.addEventListener("click", function (e) {
    var b = e.target.closest("button[data-action]");
    if (!b) { return; }
    var id = Number(b.closest("tr").dataset.id);
    if (b.dataset.action === "inspect") {
      open(id);
    } else {
      invoke("jobs_cancel", { id: id }).catch(function (err) {
        if (window.AutomationPage) { window.AutomationPage.say("Could not kill it: " + err + "."); }
      });
    }
  });

  document.getElementById("btn-jobs-inspect-close").addEventListener("click", closeInspect);

  // Listening starts before the first load, so no change is missed.
  load();
  tab.addEventListener("shown.bs.tab", load);

  window.JobsInspect = { open: open };
})();
