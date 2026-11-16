/* Jobs tab, Overview: every Trigger, Schedule and Action in one place, in
   three accordion sections (drag a section by its handle to reorder; the
   order and which sections are open are saved in config.json by app.js,
   like the Panels page). Each table can be sorted by name, by when it is
   next due (or last ran) and by on or off, filtered, and shows 30 rows a
   page. Clicking a row (or Enter on it) opens that entry's editor; hovering
   shows a summary. The quick buttons switch every trigger and schedule on
   or off, add back the demonstration entries, or delete everything (twice).

   The data comes from js/automation.js (window.AutomationPage), which
   receives every change from Rust. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;
  var page = window.AutomationPage;
  var PAGE_SIZE = 30;

  var SECTIONS = {
    triggers: { section: "trigger", word: "trigger" },
    schedules: { section: "schedule", word: "schedule" },
    actions: { section: "action", word: "action" }
  };

  var filterBox = document.getElementById("overview-filter");
  var reply = null;
  var state = {
    triggers: { sort: "name", dir: 1, page: 0 },
    schedules: { sort: "name", dir: 1, page: 0 },
    actions: { sort: "name", dir: 1, page: 0 }
  };
  var tooltips = [];
  var deleteArmed = null; // timer while Delete all waits for its second press

  function el(tag, className, text) {
    var e = document.createElement(tag);
    if (className) { e.className = className; }
    if (text !== undefined && text !== null) { e.textContent = text; }
    return e;
  }

  // ---- What each row shows --------------------------------------------------------

  function lastRun(a) {
    return a.history.length ? a.history[a.history.length - 1] : null;
  }

  // When it is next due (triggers, schedules) or last ran (actions), for
  // sorting. Nothing due sorts last.
  function dueOf(key, item) {
    if (key === "actions") {
      var last = lastRun(item);
      return last ? -last.at_ms : Infinity; // newest first
    }
    return item.enabled && item.next_ms ? item.next_ms : Infinity;
  }

  // The words the filter searches.
  function words(key, item) {
    var parts = [item.name];
    if (key === "triggers") { parts.push(item.watching, item.when); }
    if (key === "schedules") { parts.push(item.when); }
    if (key === "actions") { parts.push(item.summary); }
    return parts.concat(item.then || []).join(" ").toLowerCase();
  }

  function summary(key, item) {
    var lines = [];
    if (key === "triggers") {
      lines.push("Watches " + item.watching);
      lines.push("Fires when it " + item.when + ", checked every " + page.everyWords(item.every_seconds));
      if (item.last_error) { lines.push("Last check failed: " + item.last_error); }
      if (item.fires) { lines.push("Fired " + page.plural(item.fires, "time")); }
    }
    if (key === "schedules") {
      lines.push("Runs " + item.when);
      if (item.max_runs) { lines.push("At most " + page.plural(item.max_runs, "time")); }
    }
    if (key === "actions") {
      lines.push(item.summary);
      var last = lastRun(item);
      if (last) { lines.push("Last: " + (page.states[last.state] || page.states.failed).label + ", by " + last.by); }
    }
    lines.push(item.then && item.then.length ? "Then starts: " + item.then.join(", ") : "Starts no actions");
    return lines.join("\n");
  }

  function cells(key, item) {
    var on = el("span", "badge " + (item.enabled ? "badge-success" : "badge-neutral"), item.enabled ? "On" : "Off");
    var name = el("td", "jobs-wrap");
    name.appendChild(el("div", "jobs-name", item.name));
    var due = el("td", "jobs-wrap");
    var info = el("td", "jobs-wrap");
    if (key === "triggers") {
      name.appendChild(el("div", "jobs-subtle", item.watching));
      due.textContent = item.enabled ? (item.checking ? "checking now" : page.when(item.next_ms)) : "off";
      if (item.last_error) {
        info.appendChild(el("span", "badge badge-danger", "Could not read"));
      } else {
        info.textContent = item.last_value ? item.last_value.text : "–";
      }
    }
    if (key === "schedules") {
      name.appendChild(el("div", "jobs-subtle", item.when));
      due.textContent = !item.enabled ? "off" : item.next_ms ? page.when(item.next_ms) : "never again";
      info.textContent = item.max_runs ? item.runs + " of " + item.max_runs : String(item.runs);
    }
    if (key === "actions") {
      name.appendChild(el("div", "jobs-subtle", item.summary));
      var last = lastRun(item);
      due.textContent = last ? page.when(last.at_ms) : "not run yet";
      if (item.running.length) {
        info.appendChild(el("span", "badge badge-info", "Running"));
      } else if (last) {
        var st = page.states[last.state] || page.states.failed;
        info.appendChild(el("span", "badge " + st.badge, st.label));
      } else {
        info.textContent = "–";
      }
    }
    var onCell = el("td");
    onCell.appendChild(on);
    return [onCell, name, due, info];
  }

  // ---- Drawing ----------------------------------------------------------------------

  function clearTooltips() {
    tooltips.forEach(function (t) { t.dispose(); });
    tooltips = [];
  }

  function sorted(key, list) {
    var s = state[key];
    var text = filterBox.value.trim().toLowerCase();
    var shown = list.filter(function (item) { return !text || words(key, item).indexOf(text) >= 0; });
    shown.sort(function (a, b) {
      var x, y;
      if (s.sort === "enabled") { x = a.enabled ? 0 : 1; y = b.enabled ? 0 : 1; }
      else if (s.sort === "due") { x = dueOf(key, a); y = dueOf(key, b); }
      else { x = a.name.toLowerCase(); y = b.name.toLowerCase(); }
      if (x === y) { return a.name.localeCompare(b.name); }
      return (x < y ? -1 : 1) * s.dir;
    });
    return shown;
  }

  function pager(key, total) {
    var box = document.getElementById("overview-" + key + "-pager");
    box.textContent = "";
    var s = state[key];
    var pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
    if (s.page >= pages) { s.page = pages - 1; }
    if (total <= PAGE_SIZE) { return; }
    var from = s.page * PAGE_SIZE + 1;
    var to = Math.min(total, from + PAGE_SIZE - 1);
    var prev = el("button", "btn btn-secondary btn-sm");
    prev.type = "button";
    prev.dataset.page = "-1";
    prev.disabled = s.page === 0;
    prev.setAttribute("aria-label", "Previous page");
    prev.appendChild(el("i", "bi bi-chevron-left"));
    var next = el("button", "btn btn-secondary btn-sm");
    next.type = "button";
    next.dataset.page = "1";
    next.disabled = s.page >= pages - 1;
    next.setAttribute("aria-label", "Next page");
    next.appendChild(el("i", "bi bi-chevron-right"));
    box.appendChild(prev);
    box.appendChild(el("span", "jobs-subtle", from + " to " + to + " of " + total));
    box.appendChild(next);
  }

  function drawTable(key) {
    var list = reply ? reply[key] : [];
    var tbody = document.getElementById("overview-" + key + "-rows");
    var shown = sorted(key, list);
    var count = document.getElementById("overview-" + key + "-count");
    count.textContent = shown.length === list.length ? " (" + list.length + ")" : " (" + shown.length + " of " + list.length + ")";
    pager(key, shown.length);
    tbody.textContent = "";
    if (!shown.length) {
      var tr = el("tr");
      var td = el("td", "jobs-empty", list.length ? "Nothing matches the filter." : "None yet.");
      td.colSpan = 4;
      tr.appendChild(td);
      tbody.appendChild(tr);
    }
    var start = state[key].page * PAGE_SIZE;
    shown.slice(start, start + PAGE_SIZE).forEach(function (item) {
      var tr = el("tr", "overview-row");
      tr.tabIndex = 0;
      tr.dataset.section = SECTIONS[key].section;
      tr.dataset.name = item.name;
      tr.setAttribute("aria-label", "Open " + SECTIONS[key].word + " " + item.name);
      cells(key, item).forEach(function (c) { tr.appendChild(c); });
      tbody.appendChild(tr);
      tooltips.push(new bootstrap.Tooltip(tr, {
        title: summary(key, item),
        placement: "top",
        customClass: "jobs-tooltip",
        trigger: "hover"
      }));
    });
    document.querySelectorAll('.overview-table[data-overview="' + key + '"] .overview-sort').forEach(function (b) {
      var on = b.dataset.sort === state[key].sort;
      b.classList.toggle("active", on);
      b.setAttribute("aria-sort", on ? (state[key].dir > 0 ? "ascending" : "descending") : "none");
    });
  }

  function draw() {
    clearTooltips();
    Object.keys(SECTIONS).forEach(drawTable);
  }

  // ---- Events -----------------------------------------------------------------------

  page.onReply(function (r) {
    reply = r;
    draw();
  });

  filterBox.addEventListener("input", function () {
    Object.keys(state).forEach(function (k) { state[k].page = 0; });
    draw();
  });

  document.getElementById("jobs-overview").addEventListener("click", function (e) {
    var sortButton = e.target.closest(".overview-sort");
    var pageButton = e.target.closest("[data-page]");
    var key = e.target.closest("[data-overview]") ? e.target.closest("[data-overview]").dataset.overview : null;
    if (sortButton && key) {
      var s = state[key];
      if (s.sort === sortButton.dataset.sort) { s.dir = -s.dir; } else { s.sort = sortButton.dataset.sort; s.dir = 1; }
      draw();
      return;
    }
    if (pageButton) {
      var pagerKey = pageButton.closest(".overview-pager").id.replace("overview-", "").replace("-pager", "");
      state[pagerKey].page += Number(pageButton.dataset.page);
      draw();
      return;
    }
    var row = e.target.closest("tr.overview-row");
    if (row) {
      clearTooltips();
      page.open(row.dataset.section, row.dataset.name);
      draw();
    }
  });

  document.getElementById("jobs-overview").addEventListener("keydown", function (e) {
    var row = e.target.closest("tr.overview-row");
    if (row && e.key === "Enter") {
      e.preventDefault();
      clearTooltips();
      page.open(row.dataset.section, row.dataset.name);
      draw();
    }
  });

  document.querySelectorAll("[data-bulk]").forEach(function (button) {
    button.addEventListener("click", function () {
      var what = button.dataset.bulk;
      if (what === "delete_all" && deleteArmed === null) {
        button.classList.add("jobs-confirm");
        page.say("Press Delete all again within 4 seconds to delete every trigger, schedule and action.");
        deleteArmed = setTimeout(function () {
          deleteArmed = null;
          button.classList.remove("jobs-confirm");
          page.say("");
        }, 4000);
        return;
      }
      if (what === "delete_all") {
        clearTimeout(deleteArmed);
        deleteArmed = null;
        button.classList.remove("jobs-confirm");
      }
      invoke("automation_bulk", { what: what }).then(function (r) {
        page.apply(r);
        var done = {
          enable_all: "Every trigger and schedule is on.",
          disable_all: "Every trigger and schedule is off.",
          restore_demos: "The demonstration entries are back.",
          delete_all: "Everything was deleted."
        };
        page.say(done[what]);
      }, function (err) {
        page.say("Not done: " + err + ".");
      });
    });
  });

  // Menu search (Ctrl+K): the four views, and every trigger, schedule and
  // action by name; choosing one opens it.
  if (window.AppSearch && window.AppSearch.addSource) {
    window.AppSearch.addSource(function () {
      var views = [
        ["overview", "Jobs overview", "grid-1x2", ["dashboard", "summary", "all jobs"]],
        ["trigger", "Triggers", "lightning-charge", ["watch", "web watch", "fire"]],
        ["schedule", "Schedules", "calendar-event", ["timetable", "daily", "every"]],
        ["action", "Actions", "play-circle", ["notification", "command", "ai", "run"]]
      ].map(function (v) {
        return { label: v[1], page: "jobs", icon: v[2], keywords: v[3].concat(["jobs"]),
          run: function () { page.showView(v[0]); } };
      });
      if (!reply) { return views; }
      var items = [];
      [["triggers", "trigger", "lightning-charge"], ["schedules", "schedule", "calendar-event"], ["actions", "action", "play-circle"]].forEach(function (s) {
        reply[s[0]].forEach(function (item) {
          items.push({
            label: item.name + " (" + s[1] + ")",
            page: "jobs",
            icon: s[2],
            keywords: [s[1], item.watching || "", item.when || "", item.summary || ""].filter(Boolean),
            run: function () { page.open(s[1], item.name); }
          });
        });
      });
      return views.concat(items);
    });
  }

  // The page opens on the Overview.
  page.showView("overview");
})();
