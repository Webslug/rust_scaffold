/* Jobs tab, Queue view: start, watch, inspect and kill jobs run by the Rust
   job queue (src-tauri/src/jobs.rs). Kinds of job (command, AI prompt) are
   defined in src-tauri/src/job_kinds.rs; their input fields come from
   js/job_forms.js.

   Rust pushes every change to a job as a "jobs:changed" window event, so the
   table follows each job without polling. The Tokio runtime figures in the
   summary cannot be pushed; they are fetched when the page opens and when a
   job starts or ends. "At once" is saved in config.json as job_concurrency,
   the same setting as on the Options tab. */
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
  var form = document.getElementById("jobs-form");
  var runFields = window.JobForms.mount(document.getElementById("jobs-run-fields"), "jobs-run-");
  var limitSelect = document.getElementById("jobs-limit");
  var status = document.getElementById("jobs-status");
  var rows = document.getElementById("jobs-rows");
  var inspectCard = document.getElementById("jobs-inspect");
  var inspectTitle = document.getElementById("jobs-inspect-title");
  var inspectList = document.getElementById("jobs-inspect-list");
  var outputBox = document.getElementById("jobs-output-box");
  var outputPre = document.getElementById("jobs-output");
  var outputTitle = document.getElementById("jobs-output-title");

  var jobs = {};       // id -> job, as Rust sends it
  var rowOf = {};      // id -> { tr, cells }
  var limit = 2;
  var runtime = null;
  var selected = null; // id shown in the details card
  var message = "";    // last action result, shown before the summary

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

  function startedBy(origin) {
    if (!origin) { return "–"; }
    if (origin.by === "user") { return "you, from this page"; }
    if (origin.by === "schedule") { return "schedule " + origin.name; }
    if (origin.by === "rule") { return "rule " + origin.rule + " (schedule " + origin.schedule + ")"; }
    return origin.by;
  }

  function json(value) {
    return value === null || value === undefined ? "–" : JSON.stringify(value);
  }

  // ---- Table -------------------------------------------------------------

  function button(icon, label, action) {
    var b = document.createElement("button");
    b.type = "button";
    b.className = "btn btn-secondary btn-sm";
    b.dataset.action = action;
    b.setAttribute("aria-label", label);
    b.title = label;
    var i = document.createElement("i");
    i.className = "bi " + icon;
    b.appendChild(i);
    return b;
  }

  function makeRow(id) {
    var tr = document.createElement("tr");
    tr.dataset.id = id;
    var cells = {};
    ["name", "state", "progress", "wait", "run", "buttons"].forEach(function (key) {
      var td = document.createElement("td");
      tr.appendChild(td);
      cells[key] = td;
    });
    cells.run.className = "jobs-number jobs-narrow-hide";
    cells.wait.className = "jobs-number jobs-extra";
    cells.buttons.className = "jobs-buttons";

    cells.badge = document.createElement("span");
    cells.state.appendChild(cells.badge);

    var wrap = document.createElement("div");
    wrap.className = "jobs-progress";
    cells.bar = document.createElement("progress");
    cells.count = document.createElement("span");
    cells.count.className = "jobs-count";
    wrap.appendChild(cells.bar);
    wrap.appendChild(cells.count);
    cells.progress.appendChild(wrap);

    cells.buttons.appendChild(button("bi-search", "Inspect job " + id, "inspect"));
    cells.kill = button("bi-x-octagon", "Kill job " + id, "kill");
    cells.buttons.appendChild(cells.kill);
    return { tr: tr, cells: cells };
  }

  function updateRow(job) {
    var row = rowOf[job.id];
    if (!row) {
      row = rowOf[job.id] = makeRow(job.id);
      var empty = rows.querySelector(".jobs-empty");
      if (empty) { empty.parentNode.remove(); }
      rows.insertBefore(row.tr, rows.firstChild); // newest first
    }
    var c = row.cells;
    var state = STATES[job.state] || STATES.failed;
    c.name.textContent = "#" + job.id + " " + window.JobForms.label(job.kind);
    c.badge.className = "badge " + state.badge;
    c.badge.textContent = state.label;
    if (job.total > 0) {
      c.bar.max = job.total;
      c.bar.value = job.done;
    } else if (isFinished(job)) {
      c.bar.max = 1;
      c.bar.value = job.state === "done" ? 1 : 0;
    } else {
      c.bar.removeAttribute("value"); // total not known: an indeterminate bar
    }
    c.bar.setAttribute("aria-label", "Job " + job.id + " progress");
    // Counted progress, else the live note (a command's last line, the
    // start of an AI answer); a failure shows why.
    var failedText = (job.state === "failed" || job.state === "timed_out") ? job.message : null;
    var text = job.total > 0 ? job.done + " / " + job.total : (failedText || job.note || (isFinished(job) ? "" : "working"));
    c.count.textContent = text;
    c.count.title = text;
    c.wait.textContent = duration(job.wait_ms);
    c.run.textContent = duration(job.run_ms);
    c.kill.disabled = isFinished(job);
    row.tr.classList.toggle("jobs-selected", selected === job.id);
  }

  function renderAll() {
    rows.textContent = "";
    rowOf = {};
    var ids = Object.keys(jobs).map(Number).sort(function (a, b) { return a - b; });
    ids.forEach(function (id) { updateRow(jobs[id]); });
    if (!ids.length) {
      var tr = document.createElement("tr");
      var td = document.createElement("td");
      td.colSpan = 6;
      td.className = "jobs-empty";
      td.textContent = "No jobs yet. Add one above.";
      tr.appendChild(td);
      rows.appendChild(tr);
    }
  }

  // ---- Summary -------------------------------------------------------------

  function summary() {
    var count = { queued: 0, running: 0, done: 0, failed: 0, cancelled: 0, timed_out: 0 };
    Object.keys(jobs).forEach(function (id) {
      if (count.hasOwnProperty(jobs[id].state)) { count[jobs[id].state] += 1; }
    });
    var text = "Running " + count.running + " of " + limit + " at once, " + count.queued + " waiting. " +
      "Done " + count.done + ", killed " + count.cancelled + ", failed " + count.failed +
      (count.timed_out ? ", timed out " + count.timed_out : "") + ".";
    if (runtime) {
      text += " Tokio: " + runtime.workers + " worker threads, " + runtime.alive_tasks + " live tasks, " +
        runtime.global_queue_depth + " waiting for a thread.";
    }
    status.textContent = (message ? message + " " : "") + text;
  }

  function setLimitSelect(value) {
    limitSelect.value = String(value);
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
      limit = reply.limit;
      runtime = reply.runtime;
      setLimitSelect(limit);
      renderAll();
      summary();
      if (selected !== null) { inspect(selected); }
    }).catch(function (err) {
      status.textContent = "Could not load the jobs: " + err;
    });
  }

  // Runtime figures only, after a job starts or ends.
  function refreshRuntime() {
    invoke("jobs_list").then(function (reply) {
      runtime = reply.runtime;
      limit = reply.limit;
      summary();
    }).catch(function () {});
  }

  // ---- Details card -----------------------------------------------------------

  function inspect(id) {
    invoke("jobs_inspect", { id: id }).then(function (info) {
      var job = info.job;
      var state = STATES[job.state] || STATES.failed;
      var items = [
        ["State", state.label],
        ["Kind", job.kind],
        ["Started by", startedBy(job.origin)],
        ["Uses", uses(job)],
        ["Needs", job.resources.length ? job.resources.join(", ") : "–"],
        ["Input", json(job.input)],
        ["Given the result of", job.previous
          ? "schedule " + job.previous.schedule + ", job #" + job.previous.job + " (" + (STATES[job.previous.state] || STATES.failed).label +
            (job.previous.label ? ", " + job.previous.label : "") + "; " + job.previous.output_bytes + " bytes of output)"
          : "–"],
        ["Progress", job.total > 0 ? job.done + " of " + job.total : "not counted"],
        ["Live note", job.note || "–"],
        ["Time limit", duration(job.timeout_ms)],
        ["Queued at", clock(job.created_ms)],
        ["Started at", clock(job.started_ms)],
        ["Finished at", clock(job.finished_ms)],
        ["Waited", duration(job.wait_ms)],
        ["Ran", duration(job.run_ms)],
        ["Place in queue", info.queue_position === null ? "–" : String(info.queue_position)],
        ["Tokio task id", info.task_id === null ? "none (no task running)" : info.task_id],
        ["Task ended in Tokio", info.task_finished === null ? "–" : (info.task_finished ? "yes" : "no")],
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
      inspectTitle.textContent = "Job #" + job.id + " " + window.JobForms.label(job.kind);
      inspectCard.classList.remove("d-none");
    }, function () {
      closeInspect(); // the job was cleared
    });
  }

  function select(id) {
    var previous = selected;
    selected = id;
    [previous, id].forEach(function (other) {
      if (other !== null && rowOf[other]) {
        rowOf[other].tr.classList.toggle("jobs-selected", selected === other);
      }
    });
  }

  function closeInspect() {
    select(null);
    inspectCard.classList.add("d-none");
  }

  // ---- Events and actions -----------------------------------------------------

  tauri.event.listen("jobs:changed", function (e) {
    var job = e.payload;
    var before = jobs[job.id];
    if (before && before.seq > job.seq) { return; } // already have newer
    jobs[job.id] = job;
    updateRow(job);
    if (!before || before.state !== job.state) {
      refreshRuntime(); // started, finished or killed
    }
    summary();
    if (selected === job.id) { inspect(job.id); }
  });

  function report(text) {
    message = text;
    summary();
  }

  form.addEventListener("submit", function (e) {
    e.preventDefault();
    var job;
    try {
      job = runFields.read();
    } catch (err) {
      report("Not started: " + err.message + ".");
      return;
    }
    invoke("jobs_start", { kind: job.kind, input: job.input }).then(function (id) {
      report("Started job #" + id + ".");
    }, function (err) {
      report("Not started: " + err + ".");
    });
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
    copyText(outputPre.textContent).then(function () {
      report("Output copied.");
    }, function () {
      report("Could not copy the output.");
    });
  });

  document.getElementById("btn-jobs-cancel-all").addEventListener("click", function () {
    invoke("jobs_cancel_all").then(function (n) {
      report(n === 1 ? "Killed 1 job." : "Killed " + n + " jobs.");
    });
  });

  document.getElementById("btn-jobs-clear").addEventListener("click", function () {
    invoke("jobs_clear_finished").then(function (n) {
      message = n === 1 ? "Cleared 1 finished job." : "Cleared " + n + " finished jobs.";
      if (selected !== null && jobs[selected] && isFinished(jobs[selected])) { closeInspect(); }
      load();
    });
  });

  rows.addEventListener("click", function (e) {
    var b = e.target.closest("button[data-action]");
    if (!b) { return; }
    var id = Number(b.closest("tr").dataset.id);
    if (b.dataset.action === "inspect") {
      select(id);
      inspect(id);
      inspectCard.scrollIntoView({ block: "nearest" });
    } else {
      invoke("jobs_cancel", { id: id }).then(function () {
        report("Killed job #" + id + ".");
      }, function (err) {
        report("Job #" + id + ": " + err + ".");
      });
    }
  });

  document.getElementById("btn-jobs-inspect-close").addEventListener("click", closeInspect);

  limitSelect.addEventListener("change", function () {
    var value = Number(limitSelect.value);
    window.AppConfig.commit({ job_concurrency: value }).then(function () {
      limit = value;
      report("Now " + value + " at once (saved).");
    }, function (err) {
      setLimitSelect(limit);
      report("Not saved: " + err + ".");
    });
  });

  // Listening starts before the first load, so no change is missed.
  load();
  tab.addEventListener("shown.bs.tab", function () {
    message = "";
    load(); // also picks up an "At once" change saved on the Options tab
  });
})();
