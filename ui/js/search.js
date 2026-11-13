/* Menu search. Finds places in the app from search.json (read through Rust,
   see src-tauri/src/search.rs) plus every tab by its own name, and jumps to
   the chosen one: selects its tab, opens a closed accordion panel, scrolls
   the element into view, focuses it and outlines it briefly. Matching starts
   at three characters. The library is re-read each time the box gets focus,
   so edits to search.json need no restart. */
(function () {
  "use strict";

  var MIN_CHARS = 3;
  var MAX_RESULTS = 8;
  var HIGHLIGHT_MS = 2000;
  var ASYNC_WAIT_MS = 2000;

  var invoke = window.__TAURI__.core.invoke;
  var root = document.documentElement;
  var input = document.getElementById("app-search");
  var results = document.getElementById("app-search-results");
  var status = document.getElementById("app-search-status");
  var openButton = document.getElementById("btn-search-open");
  var helpStatus = document.getElementById("help-search-status");
  var helpProblems = document.getElementById("help-search-problems");

  var entries = [];      // usable entries, library first, then tabs
  var shown = [];        // entries currently in the list
  var active = -1;       // index into shown, or -1
  var highlighted = null;
  var highlightTimer = null;

  var STATUS_TEXT = {
    too_large: "search.json is larger than 1 MB and was not read.",
    invalid: "search.json could not be read as a search library.",
    unreadable: "search.json could not be read or created."
  };

  function pageLabel(page) {
    var label = document.querySelector("#tab-" + page + " .tab-label");
    return label ? label.textContent.trim() : page;
  }

  function lower(text) {
    return String(text).toLowerCase();
  }

  /* Every tab is searchable by its own name, even without a library entry. */
  function tabEntries() {
    return Array.prototype.map.call(document.querySelectorAll("#app-tabs .nav-link[id^='tab-']"), function (tab) {
      var icon = tab.querySelector(".bi");
      var iconName = icon ? (icon.className.match(/\bbi-([a-z0-9-]+)/) || [])[1] : null;
      return { label: pageLabel(tab.id.slice(4)), page: tab.id.slice(4), keywords: [], icon: iconName };
    });
  }

  function showLibraryState(reply, problems) {
    if (!helpStatus) { return; }
    var heading = STATUS_TEXT[reply.status];
    helpStatus.textContent = heading
      ? heading
      : reply.entries.length + " places in " + reply.path;
    helpProblems.textContent = "";
    var lines = (reply.status === "invalid" ? [] : reply.skipped).concat(problems);
    if (!lines.length) { helpProblems.classList.add("d-none"); return; }
    helpProblems.appendChild(document.createTextNode("Some search.json entries were skipped:"));
    var list = document.createElement("ul");
    list.className = "mb-0";
    lines.forEach(function (line) {
      var item = document.createElement("li");
      item.textContent = line;
      list.appendChild(item);
    });
    helpProblems.appendChild(list);
    helpProblems.classList.remove("d-none");
  }

  function load() {
    return invoke("get_search").then(function (reply) {
      var problems = [];
      var library = reply.entries.filter(function (entry) {
        if (!document.getElementById("tab-" + entry.page)) {
          problems.push("\"" + entry.label + "\": no page called \"" + entry.page + "\"");
          return false;
        }
        if (entry.target && !document.getElementById(entry.target) &&
            !document.getElementById("page-" + entry.page).hasAttribute("data-async-render")) {
          problems.push("\"" + entry.label + "\": no element with id \"" + entry.target + "\"");
          return false;
        }
        return true;
      });
      entries = library.concat(tabEntries().filter(function (tab) {
        return !library.some(function (e) { return e.page === tab.page && !e.target; });
      }));
      showLibraryState(reply, problems);
    }).catch(function (err) {
      entries = tabEntries();
      if (helpStatus) { helpStatus.textContent = "Could not load search.json: " + err; }
    });
  }

  /* ---- matching. Every word typed must match something; the best entries
     are the ones whose own label starts with what was typed. */

  function wordScore(word, entry) {
    var label = lower(entry.label);
    var best = 0;
    if (label.indexOf(word) === 0) { best = 60; }
    else if (label.split(/[\s-]+/).some(function (w) { return w.indexOf(word) === 0; })) { best = 45; }
    else if (label.indexOf(word) !== -1) { best = 25; }
    entry.keywords.forEach(function (k) {
      k = lower(k);
      if (k.indexOf(word) === 0) { best = Math.max(best, 40); }
      else if (k.indexOf(word) !== -1) { best = Math.max(best, 20); }
    });
    if (lower(pageLabel(entry.page)).indexOf(word) === 0) { best = Math.max(best, 15); }
    return best;
  }

  function match(query) {
    var words = query.split(/\s+/).filter(Boolean);
    var scored = [];
    entries.forEach(function (entry, order) {
      var total = 0;
      for (var i = 0; i < words.length; i++) {
        var s = wordScore(words[i], entry);
        if (!s) { return; }
        total += s;
      }
      if (lower(entry.label).indexOf(query) === 0) { total += 50; }
      if (!entry.target) { total += 5; }   // the page itself before its parts
      scored.push({ entry: entry, score: total, order: order });
    });
    scored.sort(function (a, b) { return b.score - a.score || a.order - b.order; });
    return scored.slice(0, MAX_RESULTS).map(function (s) { return s.entry; });
  }

  /* ---- the drop-down list */

  function close() {
    results.hidden = true;
    results.textContent = "";
    shown = [];
    active = -1;
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
  }

  function setActive(index) {
    var items = results.children;
    if (active >= 0 && items[active]) { items[active].setAttribute("aria-selected", "false"); }
    active = index;
    if (active >= 0 && items[active]) {
      items[active].setAttribute("aria-selected", "true");
      items[active].scrollIntoView({ block: "nearest" });
      input.setAttribute("aria-activedescendant", items[active].id);
    } else {
      input.removeAttribute("aria-activedescendant");
    }
  }

  function render() {
    var query = lower(input.value.trim());
    if (query.length < MIN_CHARS) { close(); status.textContent = ""; return; }
    shown = match(query);
    results.textContent = "";
    active = -1;
    input.removeAttribute("aria-activedescendant");
    if (!shown.length) {
      var empty = document.createElement("li");
      empty.className = "app-search-empty";
      empty.textContent = "No matches";
      results.appendChild(empty);
    }
    shown.forEach(function (entry, i) {
      var item = document.createElement("li");
      item.className = "app-search-option";
      item.id = "app-search-option-" + i;
      item.setAttribute("role", "option");
      item.setAttribute("aria-selected", "false");
      var icon = document.createElement("i");
      icon.className = "bi bi-" + (entry.icon || "arrow-return-right");
      icon.setAttribute("aria-hidden", "true");
      var label = document.createElement("span");
      label.className = "app-search-label";
      label.textContent = entry.label;
      item.appendChild(icon);
      item.appendChild(label);
      var page = pageLabel(entry.page);
      if (page !== entry.label) {
        var where = document.createElement("small");
        where.className = "app-search-page";
        where.textContent = page;
        item.appendChild(where);
      }
      // mousedown would blur the input and close the list before the click.
      item.addEventListener("mousedown", function (e) { e.preventDefault(); });
      item.addEventListener("click", function () { go(entry); });
      results.appendChild(item);
    });
    results.hidden = false;
    input.setAttribute("aria-expanded", "true");
    status.textContent = shown.length ? shown.length + (shown.length === 1 ? " result" : " results") : "No matches";
  }

  /* ---- jumping */

  function clearHighlight() {
    clearTimeout(highlightTimer);
    if (highlighted) { highlighted.classList.remove("search-hit"); }
    highlighted = null;
  }

  function isFocusable(el) {
    return el.matches("input, select, textarea, button, a[href], [tabindex]") && !el.disabled;
  }

  function reveal(entry) {
    var pane = document.getElementById("page-" + entry.page);
    var el = entry.target ? document.getElementById(entry.target) : pane;
    if (!el) { return; }

    // Open a closed accordion panel that holds (or is) the target.
    var collapse = el.classList.contains("accordion-collapse") ? el : el.closest(".accordion-collapse");
    if (collapse && !collapse.classList.contains("show")) {
      bootstrap.Collapse.getOrCreateInstance(collapse, { toggle: false }).show();
    }

    var box = el.closest(".option-row, .accordion-item, .card") || el;
    clearHighlight();
    if (el !== pane) {
      highlighted = box;
      box.classList.add("search-hit");
      highlightTimer = setTimeout(clearHighlight, HIGHLIGHT_MS);
    }
    box.scrollIntoView({ block: "start" });

    var focusEl = isFocusable(el) ? el : box;
    if (!isFocusable(focusEl)) {
      focusEl.setAttribute("tabindex", "-1");
      focusEl.addEventListener("blur", function () { focusEl.removeAttribute("tabindex"); }, { once: true });
    }
    focusEl.focus({ preventScroll: true });
  }

  function go(entry) {
    close();
    input.value = "";
    status.textContent = "";
    var tab = document.getElementById("tab-" + entry.page);
    var pane = document.getElementById("page-" + entry.page);
    if (!tab || !pane) { return; }
    var wasActive = tab.classList.contains("active");
    // Pages that draw their content after being shown (the Options tab)
    // announce it with a page:rendered event; wait for that, with a limit.
    if (!wasActive && pane.hasAttribute("data-async-render")) {
      var done = false;
      var finish = function () {
        if (done) { return; }
        done = true;
        document.removeEventListener("page:rendered", onRendered);
        reveal(entry);
      };
      var onRendered = function (e) { if (e.detail && e.detail.page === entry.page) { finish(); } };
      document.addEventListener("page:rendered", onRendered);
      setTimeout(finish, ASYNC_WAIT_MS);
      bootstrap.Tab.getOrCreateInstance(tab).show();
      return;
    }
    bootstrap.Tab.getOrCreateInstance(tab).show();
    reveal(entry);
  }

  /* ---- events */

  input.addEventListener("focus", function () { load().then(render); });
  input.addEventListener("input", render);
  input.addEventListener("blur", close);

  input.addEventListener("keydown", function (e) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!shown.length) { return; }
      e.preventDefault();
      var step = e.key === "ArrowDown" ? 1 : -1;
      setActive((active + step + shown.length) % shown.length);
    } else if (e.key === "Enter") {
      if (!shown.length) { return; }
      e.preventDefault();
      go(shown[active >= 0 ? active : 0]);
    } else if (e.key === "Escape") {
      if (!results.hidden) { e.preventDefault(); close(); }
      else if (input.value) { e.preventDefault(); input.value = ""; status.textContent = ""; }
    }
  });

  function openSearch() {
    if (root.getAttribute("data-menu") !== "vertical") { return; }
    if (window.AppMenu) { window.AppMenu.setCollapsed(false); }
    input.focus();
    input.select();
  }

  openButton.addEventListener("click", openSearch);

  // Ctrl+K moves to the search box (vertical menu only).
  document.addEventListener("keydown", function (e) {
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && lower(e.key) === "k") {
      if (root.getAttribute("data-menu") !== "vertical") { return; }
      e.preventDefault();
      openSearch();
    }
  });

  load();
})();
