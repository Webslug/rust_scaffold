/* Options tab. Draws one control per entry in options.json (checked by Rust,
   see src-tauri/src/options.rs) and edits the matching config.json value.
   options.json is read again each time the tab opens, so edits to it show up
   without a rebuild. Changes are staged until Save; only changed keys are
   saved, through window.AppConfig in app.js. Opening the tab again discards
   unsaved changes. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;

  var STATUS_TEXT = {
    too_large: "options.json is larger than 1 MB, so it was not loaded.",
    invalid: "options.json could not be used:",
    unreadable: "options.json could not be read or created."
  };

  var tab = document.getElementById("tab-options");
  var groupsBox = document.getElementById("options-groups");
  var problems = document.getElementById("options-problems");
  var statusLine = document.getElementById("options-status");
  var saveButton = document.getElementById("btn-options-save");
  var revertButton = document.getElementById("btn-options-revert");

  var specs = [];        // usable entries from options.json
  var original = {};     // key -> value when the tab was drawn
  var draft = {};        // key -> value as currently edited
  var invalid = {};      // key -> true while its field holds a bad value
  var setters = {};      // key -> function(value) that updates the control
  var busy = false;

  function same(a, b) { return JSON.stringify(a) === JSON.stringify(b); }

  function choicesOf(spec) {
    if (spec.choices === "themes") {
      return window.AppTheme.list().map(function (t) { return { value: t.id, label: t.label }; });
    }
    return spec.choices.map(function (c) {
      return typeof c === "string" ? { value: c, label: c } : c;
    });
  }

  function kindOf(control) {
    if (control === "switch" || control === "check") { return "bool"; }
    if (control === "number" || control === "range") { return "number"; }
    if (control === "input" || control === "text") { return "text"; }
    return "choice";
  }

  function fits(spec, value) {
    var kind = kindOf(spec.control);
    if (kind === "bool") { return typeof value === "boolean"; }
    if (kind === "number") { return typeof value === "number" && isFinite(value); }
    if (kind === "text") { return typeof value === "string"; }
    return choicesOf(spec).some(function (c) { return same(c.value, value); });
  }

  /* The value to show: the saved one if it suits the control, otherwise the
     entry's default, otherwise an empty value of the right kind. */
  function startValue(spec, config) {
    var builtIn = spec.key !== "extras" && Object.prototype.hasOwnProperty.call(config, spec.key);
    var saved = builtIn ? config[spec.key] : config.extras[spec.key];
    if (saved !== undefined && fits(spec, saved)) { return saved; }
    if (spec["default"] !== undefined) { return spec["default"]; }
    var kind = kindOf(spec.control);
    if (kind === "bool") { return false; }
    if (kind === "number") { return spec.min !== undefined ? spec.min : 0; }
    if (kind === "text") { return ""; }
    return choicesOf(spec)[0].value;
  }

  function changedKeys() {
    return Object.keys(draft).filter(function (key) {
      return !same(draft[key], original[key]);
    });
  }

  function refreshButtons() {
    var dirty = changedKeys().length > 0;
    var bad = Object.keys(invalid).length > 0;
    saveButton.disabled = busy || !dirty || bad || !window.AppConfig.canSave();
    revertButton.disabled = busy || !dirty;
  }

  function setDraft(key, value) {
    draft[key] = value;
    statusLine.textContent = "";
    refreshButtons();
  }

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) { node.className = className; }
    if (text !== undefined) { node.textContent = text; }
    return node;
  }

  function sizeClass(spec, fallback) {
    return "option-size-" + (spec.size || fallback);
  }

  /* ---- one builder per control type. Each returns the row element and
     registers a setter so Revert and Restore defaults can update it. */

  function buildCheck(spec, id, row) {
    var wrap = el("div", spec.control === "switch" ? "form-check form-switch" : "form-check");
    var input = el("input", "form-check-input");
    input.type = "checkbox";
    input.id = id;
    if (spec.control === "switch") { input.setAttribute("role", "switch"); }
    input.disabled = spec.readonly;
    input.addEventListener("change", function () { setDraft(spec.key, input.checked); });
    var label = el("label", "form-check-label", spec.label);
    label.htmlFor = id;
    wrap.appendChild(input);
    wrap.appendChild(label);
    row.appendChild(wrap);
    setters[spec.key] = function (v) { input.checked = v; };
  }

  function addLabel(spec, id, row) {
    var label = el("label", "form-label", spec.label);
    label.htmlFor = id;
    row.appendChild(label);
  }

  function buildText(spec, id, row) {
    addLabel(spec, id, row);
    var multi = spec.control === "text";
    var input = el(multi ? "textarea" : "input", "form-control " + sizeClass(spec, multi ? "full" : "md"));
    if (!multi) { input.type = "text"; }
    else { input.rows = spec.rows || 3; }
    input.id = id;
    input.readOnly = spec.readonly;
    if (spec.placeholder) { input.placeholder = spec.placeholder; }
    input.maxLength = 4000;
    input.autocomplete = "off";
    input.addEventListener("input", function () { setDraft(spec.key, input.value); });
    row.appendChild(input);
    setters[spec.key] = function (v) { input.value = v; };
  }

  function buildNumber(spec, id, row) {
    addLabel(spec, id, row);
    var input = el("input", "form-control " + sizeClass(spec, "sm"));
    input.type = "number";
    input.id = id;
    input.readOnly = spec.readonly;
    if (spec.min !== undefined) { input.min = spec.min; }
    if (spec.max !== undefined) { input.max = spec.max; }
    if (spec.step !== undefined) { input.step = spec.step; }
    input.addEventListener("input", function () {
      var n = Number(input.value);
      var ok = input.value.trim() !== "" && isFinite(n) &&
        (spec.min === undefined || n >= spec.min) && (spec.max === undefined || n <= spec.max);
      input.classList.toggle("is-invalid", !ok);
      if (ok) { delete invalid[spec.key]; setDraft(spec.key, n); }
      else { invalid[spec.key] = true; refreshButtons(); }
    });
    row.appendChild(input);
    setters[spec.key] = function (v) {
      input.value = v;
      input.classList.remove("is-invalid");
      delete invalid[spec.key];
    };
  }

  function buildRange(spec, id, row) {
    addLabel(spec, id, row);
    var wrap = el("div", "option-range " + sizeClass(spec, "md"));
    var input = el("input", "form-range");
    input.type = "range";
    input.id = id;
    input.min = spec.min;
    input.max = spec.max;
    input.step = spec.step !== undefined ? spec.step : "any";
    input.disabled = spec.readonly;
    var shown = el("output", "option-range-value");
    shown.htmlFor = id;
    input.addEventListener("input", function () {
      shown.textContent = input.value;
      setDraft(spec.key, Number(input.value));
    });
    wrap.appendChild(input);
    wrap.appendChild(shown);
    row.appendChild(wrap);
    setters[spec.key] = function (v) { input.value = v; shown.textContent = v; };
  }

  function buildSelect(spec, id, row) {
    addLabel(spec, id, row);
    var choices = choicesOf(spec);
    var select = el("select", "form-select " + sizeClass(spec, "md"));
    select.id = id;
    select.disabled = spec.readonly;
    choices.forEach(function (c, i) {
      var option = el("option", "", c.label);
      option.value = String(i);   // index, so number values keep their type
      select.appendChild(option);
    });
    select.addEventListener("change", function () {
      setDraft(spec.key, choices[Number(select.value)].value);
    });
    row.appendChild(select);
    setters[spec.key] = function (v) {
      choices.forEach(function (c, i) { if (same(c.value, v)) { select.value = String(i); } });
    };
  }

  function buildRadio(spec, id, row) {
    var group = el("fieldset", "");
    group.appendChild(el("legend", "form-label", spec.label));
    var inputs = choicesOf(spec).map(function (c, i) {
      var wrap = el("div", "form-check");
      var input = el("input", "form-check-input");
      input.type = "radio";
      input.name = id;
      input.id = id + "-" + i;
      input.disabled = spec.readonly;
      input.addEventListener("change", function () { if (input.checked) { setDraft(spec.key, c.value); } });
      var label = el("label", "form-check-label", c.label);
      label.htmlFor = input.id;
      wrap.appendChild(input);
      wrap.appendChild(label);
      group.appendChild(wrap);
      return { input: input, value: c.value };
    });
    row.appendChild(group);
    setters[spec.key] = function (v) {
      inputs.forEach(function (r) { r.input.checked = same(r.value, v); });
    };
  }

  function buildList(spec, id, row) {
    var heading = el("p", "form-label", spec.label);
    heading.id = id + "-label";
    row.appendChild(heading);
    var list = el("div", "list-group " + sizeClass(spec, "md"));
    list.setAttribute("role", "group");
    list.setAttribute("aria-labelledby", heading.id);
    var items = choicesOf(spec).map(function (c) {
      var item = el("button", "list-group-item list-group-item-action", c.label);
      item.type = "button";
      item.disabled = spec.readonly;
      item.addEventListener("click", function () { setters[spec.key](c.value); setDraft(spec.key, c.value); });
      list.appendChild(item);
      return { item: item, value: c.value };
    });
    row.appendChild(list);
    setters[spec.key] = function (v) {
      items.forEach(function (i) {
        var on = same(i.value, v);
        i.item.classList.toggle("active", on);
        i.item.setAttribute("aria-pressed", on ? "true" : "false");
      });
    };
  }

  function buildButton(spec, id, row) {
    var button = el("button", "btn btn-secondary btn-sm", spec.label);
    button.type = "button";
    button.id = id;
    button.addEventListener("click", function () {
      if (spec.action === "revert") { revert(); }
      else if (spec.action === "defaults") { restoreDefaults(); }
    });
    row.appendChild(button);
  }

  var BUILDERS = {
    "switch": buildCheck, check: buildCheck, input: buildText, text: buildText,
    number: buildNumber, range: buildRange, select: buildSelect, radio: buildRadio,
    list: buildList, button: buildButton
  };

  /* ---- actions */

  function revert() {
    Object.keys(original).forEach(function (key) {
      draft[key] = original[key];
      setters[key](original[key]);
    });
    statusLine.textContent = "Changes discarded.";
    refreshButtons();
  }

  /* Only entries with a default in options.json; built-in settings have
     their defaults in Rust and are left alone. */
  function restoreDefaults() {
    specs.forEach(function (spec) {
      if (!spec.key || spec.readonly || spec["default"] === undefined) { return; }
      draft[spec.key] = spec["default"];
      setters[spec.key](spec["default"]);
    });
    statusLine.textContent = "Defaults restored. Press Save to keep them.";
    refreshButtons();
  }

  function save() {
    var keys = changedKeys();
    if (!keys.length) { return; }
    var changes = {};
    keys.forEach(function (key) { changes[key] = draft[key]; });
    busy = true;
    refreshButtons();
    window.AppConfig.commit(changes).then(function () {
      keys.forEach(function (key) { original[key] = draft[key]; });
      statusLine.textContent = "Saved.";
    }, function (err) {
      statusLine.textContent = "Not saved: " + err;
    }).then(function () {
      busy = false;
      refreshButtons();
    });
  }

  /* ---- drawing */

  function showProblems(reply) {
    problems.textContent = "";
    var lines = reply.skipped.slice();
    var heading = STATUS_TEXT[reply.status];
    if (!heading && lines.length) { heading = "Some entries in options.json were skipped:"; }
    if (!heading) { problems.classList.add("d-none"); return; }
    problems.appendChild(el("span", "", heading));
    if (lines.length) {
      var list = el("ul", "option-skipped");
      lines.forEach(function (line) { list.appendChild(el("li", "", line)); });
      problems.appendChild(list);
    }
    problems.classList.remove("d-none");
  }

  function render(reply) {
    var config = window.AppConfig.get();
    specs = reply.options;
    original = {};
    draft = {};
    invalid = {};
    setters = {};
    groupsBox.textContent = "";
    showProblems(reply);
    if (!config) { return; }

    var bodies = {};
    specs.forEach(function (spec, index) {
      var name = spec.group || "Options";
      if (!bodies[name]) {
        var card = el("section", "card card-custom");
        var header = el("div", "card-header");
        header.appendChild(el("h2", "h6 card-header-title mb-0", name));
        card.appendChild(header);
        bodies[name] = el("div", "card-body");
        card.appendChild(bodies[name]);
        groupsBox.appendChild(card);
      }
      var id = "opt-" + (spec.key || "button-" + index);
      var row = el("div", "option-row");
      BUILDERS[spec.control](spec, id, row);
      if (spec.help) {
        var help = el("small", "form-text", spec.help);
        help.id = id + "-help";
        row.appendChild(help);
        var target = row.querySelector("input, select, textarea, .list-group, fieldset, button");
        if (target) { target.setAttribute("aria-describedby", help.id); }
      }
      bodies[name].appendChild(row);
      if (spec.key) {
        var value = startValue(spec, config);
        original[spec.key] = value;
        draft[spec.key] = value;
        setters[spec.key](value);
      }
    });
    statusLine.textContent = window.AppConfig.canSave() ? "" : "Saving is disabled; see the warning above.";
    refreshButtons();
  }

  function load() {
    invoke("get_options").then(render).catch(function (err) {
      groupsBox.textContent = "";
      problems.textContent = "Could not load options: " + err;
      problems.classList.remove("d-none");
    });
  }

  saveButton.addEventListener("click", save);
  revertButton.addEventListener("click", revert);
  tab.addEventListener("shown.bs.tab", load);
})();
