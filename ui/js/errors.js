/* Errors tab, and reporting of JavaScript errors to Rust.

   Loaded before every other script, so it also catches errors in them.
   Rust keeps the list of warnings and errors (src-tauri/src/logging.rs) and
   pushes each new one as a "log:entry" window event the moment it happens,
   so nothing is polled. The page loads the whole list at startup and each
   time it is opened. */
(function () {
  "use strict";

  var tauri = window.__TAURI__;
  var invoke = tauri.core.invoke;

  // ---- JavaScript errors -> Rust -------------------------------------------

  // At most this many UI errors are reported per 10 seconds, so an error that
  // repeats on every frame cannot flood the list.
  var REPORT_LIMIT = 20;
  var REPORT_WINDOW_MS = 10000;
  var reportTimes = [];

  function report(message, location) {
    var now = Date.now();
    reportTimes = reportTimes.filter(function (t) { return now - t < REPORT_WINDOW_MS; });
    if (reportTimes.length >= REPORT_LIMIT) {
      return;
    }
    reportTimes.push(now);
    invoke("report_ui_error", {
      message: String(message).slice(0, 2000),
      location: location || "unknown location"
    }).catch(function () {});
  }

  window.addEventListener("error", function (e) {
    var where = e.filename ? e.filename.replace(/^.*\//, "") + ":" + e.lineno + ":" + e.colno : "";
    report(e.message || "Script error", where);
  });

  window.addEventListener("unhandledrejection", function (e) {
    var reason = e.reason;
    report("Unhandled promise rejection: " + (reason && reason.message ? reason.message : reason), "");
  });

  // ---- Errors page -----------------------------------------------------------

  var LEVELS = {
    error: { label: "Error", badge: "badge-danger" },
    warn: { label: "Warning", badge: "badge-warning" }
  };

  var tab = document.getElementById("tab-errors");
  var badge = document.getElementById("errors-badge");
  var rows = document.getElementById("errors-rows");
  var filter = document.getElementById("errors-filter");
  var status = document.getElementById("errors-status");
  var note = document.getElementById("errors-note");

  var entries = []; // oldest first, as Rust keeps them
  var max = 100;
  var unseen = 0;

  function isOpen() {
    return tab.classList.contains("active");
  }

  function cell(text, className) {
    var td = document.createElement("td");
    if (className) {
      td.className = className;
    }
    td.textContent = text;
    return td;
  }

  function render() {
    var wanted = filter.value;
    var shown = entries.filter(function (e) { return wanted === "all" || e.level === wanted; });
    rows.textContent = "";
    for (var i = shown.length - 1; i >= 0; i--) {
      var entry = shown[i];
      var level = LEVELS[entry.level] || LEVELS.error;
      var tr = document.createElement("tr");
      tr.appendChild(cell(entry.time, "errors-time"));
      var levelCell = cell("", "");
      var span = document.createElement("span");
      span.className = "badge " + level.badge;
      span.textContent = level.label;
      levelCell.appendChild(span);
      tr.appendChild(levelCell);
      tr.appendChild(cell(entry.source, "errors-source"));
      tr.appendChild(cell(entry.message, "errors-message"));
      rows.appendChild(tr);
    }
    if (!shown.length) {
      var empty = document.createElement("tr");
      var td = cell(entries.length ? "Nothing matches this filter." : "No errors or warnings since the app started.", "errors-empty");
      td.colSpan = 4;
      empty.appendChild(td);
      rows.appendChild(empty);
    }
    status.textContent = entries.length + " kept, newest " + max + " at most";
  }

  function showBadge() {
    if (unseen > 0 && !isOpen()) {
      badge.textContent = unseen > 99 ? "99+" : String(unseen);
      badge.classList.remove("d-none");
    } else {
      badge.classList.add("d-none");
    }
  }

  function load() {
    return invoke("get_errors").then(function (reply) {
      entries = reply.entries;
      max = reply.max;
      note.textContent = reply.log_file
        ? "Kept in memory since the app started. Everything from debug level up is also written to " + reply.log_file + ", which is cut to its newest half when it passes 1 MB."
        : "Kept in memory since the app started. No log file is written (DEBUG_MODE is false in src-tauri/src/lib.rs).";
      render();
    }).catch(function (err) {
      status.textContent = "Could not load the list: " + err;
    });
  }

  function add(entry) {
    for (var i = entries.length - 1; i >= 0; i--) {
      if (entries[i].id === entry.id) {
        return; // already loaded with the full list
      }
    }
    entries.push(entry);
    while (entries.length > max) {
      entries.shift();
    }
    if (isOpen()) {
      render();
    } else {
      unseen += 1;
      showBadge();
    }
  }

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

  // Listen before loading, so nothing recorded in between is missed; add()
  // skips entries the full list already holds.
  tauri.event.listen("log:entry", function (e) { add(e.payload); });
  load().then(function () {
    unseen = entries.length;
    showBadge();
  });

  tab.addEventListener("shown.bs.tab", function () {
    unseen = 0;
    showBadge();
    load(); // also picks up a new error_log_max saved on the Options tab
  });

  filter.addEventListener("change", render);

  document.getElementById("btn-errors-clear").addEventListener("click", function () {
    invoke("clear_errors").then(function () {
      entries = [];
      render();
    });
  });

  document.getElementById("btn-errors-copy").addEventListener("click", function () {
    var text = entries.map(function (e) {
      return e.time + " " + (LEVELS[e.level] || LEVELS.error).label + " " + e.source + ": " + e.message;
    }).join("\n");
    copyText(text).then(function () {
      status.textContent = "Copied " + entries.length + " entries";
    }, function (err) {
      status.textContent = "Copy failed: " + err.message;
    });
  });

  document.getElementById("btn-errors-test").addEventListener("click", function () {
    invoke("log_test_entry", { level: "error" });
  });
})();
