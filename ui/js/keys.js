/* Shortcut keys from keys.json in the settings folder (src-tauri/src/keys.rs).

   Rust checks the file and sends every key in one standard form, such as
   "Ctrl+Alt+Shift+K", "/" or "Alt+1". Here each key press is written in the
   same form and looked up. The actions themselves live in this file, so the
   file can only choose among them, never run code of its own.

   While you type in a text box, keys without Ctrl or Alt type as normal;
   only leave-field still works. A key another script has already handled
   (for example Escape closing the search list) is left alone.

   The file is read at start and again each time the Help page opens, which
   also redraws the cheat sheet there. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;
  var helpTab = document.getElementById("tab-help");
  var sheet = document.getElementById("help-keys-rows");
  var sheetStatus = document.getElementById("help-keys-status");
  var sheetProblems = document.getElementById("help-keys-problems");

  var bindings = [];
  var actionOf = {}; // standard key -> action

  var STATUS_TEXT = {
    recreated: "keys.json was larger than 64 KB, so it was treated as corrupt, kept as keys.json.corrupt and replaced with defaults.",
    too_large: "keys.json is larger than 64 KB and could not be replaced, so it was not read.",
    invalid: "keys.json could not be read as a list of shortcut keys.",
    unreadable: "keys.json could not be read or created."
  };

  var TEXT_INPUTS = ["text", "search", "email", "url", "tel", "password", "number"];

  function isTyping(el) {
    if (!el) { return false; }
    if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") { return true; }
    return el.tagName === "INPUT" && TEXT_INPUTS.indexOf(el.type) !== -1;
  }

  // The key press in the same form Rust uses. Letters are upper case and
  // carry Shift; other characters already show it ("?" rather than Shift+/).
  function keyName(e) {
    var key = e.key === " " ? "Space" : e.key;
    var shift = e.shiftKey;
    if (key.length === 1) {
      if (/^[a-z]$/i.test(key)) {
        key = key.toUpperCase();
      } else {
        shift = false;
      }
    }
    return (e.ctrlKey ? "Ctrl+" : "") + (e.altKey ? "Alt+" : "") + (shift ? "Shift+" : "") + key;
  }

  function pageTab(action) {
    return document.getElementById("tab-" + action.slice(5));
  }

  function pageLabel(tab) {
    var label = tab.querySelector(".tab-label");
    return label ? label.textContent.trim() : tab.id.slice(4);
  }

  /* ---- actions. Each returns true when it did something. */

  function run(action, focused) {
    if (action === "focus-search") {
      return Boolean(window.AppSearch && window.AppSearch.open());
    }
    if (action === "leave-field") {
      if (!isTyping(focused)) { return false; }
      focused.blur();
      return true;
    }
    var tab = pageTab(action);
    if (!tab) { return false; }
    bootstrap.Tab.getOrCreateInstance(tab).show();
    tab.focus();
    return true;
  }

  document.addEventListener("keydown", function (e) {
    if (e.defaultPrevented || e.isComposing || e.metaKey) { return; }
    var action = actionOf[keyName(e)];
    if (!action) { return; }
    var focused = document.activeElement;
    if (isTyping(focused) && !e.ctrlKey && !e.altKey && action !== "leave-field") { return; }
    if (run(action, focused)) { e.preventDefault(); }
  });

  /* ---- cheat sheet on the Help page */

  function describe(action) {
    if (action === "focus-search") { return "Move to the search box (vertical menu)"; }
    if (action === "leave-field") { return "Leave a text box"; }
    var tab = pageTab(action);
    return tab ? "Open " + pageLabel(tab) : "Open " + action.slice(5) + " (no such page)";
  }

  function keyCell(key) {
    var td = document.createElement("td");
    // "Ctrl++" splits into "Ctrl" and "+".
    var parts = key === "+" ? ["+"] : key.replace(/\+\+$/, "+\u0000").split("+");
    parts.forEach(function (part, i) {
      if (i) { td.appendChild(document.createTextNode(" + ")); }
      var kbd = document.createElement("kbd");
      kbd.textContent = part === "\u0000" ? "+" : part;
      td.appendChild(kbd);
    });
    return td;
  }

  function drawSheet(reply) {
    if (!sheet) { return; }
    sheet.textContent = "";
    bindings.forEach(function (b) {
      var tr = document.createElement("tr");
      tr.appendChild(keyCell(b.key));
      var td = document.createElement("td");
      td.textContent = describe(b.action);
      tr.appendChild(td);
      sheet.appendChild(tr);
    });
    if (!bindings.length) {
      var tr = document.createElement("tr");
      var td = document.createElement("td");
      td.colSpan = 2;
      td.textContent = "No shortcut keys are set.";
      tr.appendChild(td);
      sheet.appendChild(tr);
    }
    sheetStatus.textContent = STATUS_TEXT[reply.status] || (bindings.length + " keys from " + reply.path);

    sheetProblems.textContent = "";
    if (!reply.skipped.length) { sheetProblems.classList.add("d-none"); return; }
    sheetProblems.appendChild(document.createTextNode(
      reply.status === "invalid" ? "keys.json was not used:" : "Some keys.json bindings were skipped:"));
    var list = document.createElement("ul");
    list.className = "mb-0";
    reply.skipped.forEach(function (line) {
      var item = document.createElement("li");
      item.textContent = line;
      list.appendChild(item);
    });
    sheetProblems.appendChild(list);
    sheetProblems.classList.remove("d-none");
  }

  function load() {
    invoke("get_keys").then(function (reply) {
      bindings = reply.bindings;
      actionOf = {};
      bindings.forEach(function (b) { actionOf[b.key] = b.action; });
      drawSheet(reply);
    }).catch(function (err) {
      if (sheetStatus) { sheetStatus.textContent = "Could not load keys.json: " + err; }
    });
  }

  load();
  helpTab.addEventListener("shown.bs.tab", load);
})();
