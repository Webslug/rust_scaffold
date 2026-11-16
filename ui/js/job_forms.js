/* Input fields for each kind of Action, used by the Action editor
   (js/automation.js). The job kinds are defined in
   src-tauri/src/job_kinds.rs, "switch" in src-tauri/src/automation.rs;
   Rust checks every input again before an Action is saved, so these forms
   only help to fill it in.

   window.JobForms.mount(container, prefix) returns an object with
   set(kind, input) and read() -> { kind, input } (read throws an Error with
   a message for people when a field cannot be read). A kind without a form
   here gets a plain JSON box, so new kinds work before they get a form.
   window.JobForms.names is set by js/automation.js: names(section) lists
   the Triggers, Schedules or Actions a "switch" can choose from. */
(function () {
  "use strict";

  var PROVIDERS = [
    { value: "ollama", label: "Ollama (local)", model: "qwen3:1.7b", local: true },
    { value: "llama_cpp", label: "llama.cpp (local)", model: "optional", local: true },
    { value: "claude_code", label: "Claude Code", model: "optional, for example sonnet" },
    { value: "codex", label: "Codex", model: "optional, the tool's default" },
    { value: "grok_build", label: "GrokBuild", model: "optional, the tool's default" }
  ];

  var KIND_LABELS = {
    notify: "Notification", command: "Command (bash)", ai: "AI prompt", "switch": "Switch on or off",
    delay: "Delay a schedule", set_count: "Change a count", "delete": "Delete an entry", fire: "Fire a trigger",
    log: "Write to the log", clear_errors: "Clear the Errors list", app: "Show, hide, quit or restart the app"
  };
  // The order of the Kind list.
  var KIND_ORDER = ["notify", "command", "ai", "switch", "fire", "delay", "set_count", "delete", "log", "clear_errors", "app"];

  var PLACEHOLDERS = "{value.text} and {value.number} (the trigger's reading), {previous.text} and {previous.number} " +
    "(its reading before the last change), {output} (what the action before this one printed or answered) and {from} (what started it). " +
    "Run gives them the latest reading of the trigger that starts this action; a schedule gives none";

  var ENDPOINTS = {
    ollama: "http://127.0.0.1:11434/api/chat",
    llama_cpp: "http://127.0.0.1:8080/v1/chat/completions"
  };

  function el(tag, className, text) {
    var e = document.createElement(tag);
    if (className) { e.className = className; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  // A labelled field; `wide` spans the whole row.
  function field(id, label, control, wide) {
    var box = el("div", wide ? "jobs-wide" : "");
    var l = el("label", "form-label", label);
    l.htmlFor = id;
    control.id = id;
    box.appendChild(l);
    box.appendChild(control);
    return box;
  }

  function input(placeholder) {
    var i = el("input", "form-control");
    i.type = "text";
    i.autocomplete = "off";
    i.spellcheck = false;
    if (placeholder) { i.placeholder = placeholder; }
    return i;
  }

  function textarea(rows, placeholder) {
    var t = el("textarea", "form-control jobs-code");
    t.rows = rows;
    t.spellcheck = false;
    if (placeholder) { t.placeholder = placeholder; }
    return t;
  }

  function trimmedOrNull(value) {
    var v = value.trim();
    return v === "" ? null : v;
  }

  // ---- One form per kind ---------------------------------------------------------

  var FORMS = {
    command: function (box, p) {
      var script = textarea(4, "df -h /\n~/scripts/backup.sh");
      var folder = input("Home folder (any folder; ~ works)");
      box.appendChild(field(p + "script", "Script (bash; several lines are fine)", script, true));
      var row = el("div", "jobs-fields");
      row.appendChild(field(p + "folder", "Run in folder", folder));
      box.appendChild(row);
      box.appendChild(el("p", "jobs-subtle jobs-form-note",
        "It succeeds when the exit code is 0. What it prints is its output, which the next action gets. " +
        "It is given $JOB_VALUE and $JOB_PREVIOUS (the trigger's reading now and before, as JSON), $JOB_OUTPUT " +
        "(what the action before printed; all of it in the file $JOB_OUTPUT_FILE) and $JOB_FROM. Use them in quotes, for example \"$JOB_OUTPUT\"."));
      return {
        set: function (v) {
          script.value = v.script || "";
          folder.value = v.folder || "";
        },
        read: function () {
          if (!script.value.trim()) { throw new Error("the script is empty"); }
          return { script: script.value, folder: trimmedOrNull(folder.value) };
        }
      };
    },

    ai: function (box, p) {
      var provider = el("select", "form-select");
      PROVIDERS.forEach(function (pr) {
        var o = el("option", "", pr.label);
        o.value = pr.value;
        provider.appendChild(o);
      });
      var model = input();
      // Ollama's installed models, without image (vision) models; "Other"
      // shows the text box.
      var modelList = el("select", "form-select d-none");
      modelList.setAttribute("aria-label", "Model");
      var endpoint = input();
      var folder = input("None: an empty temporary folder");
      var prompt = textarea(4, "What should the AI do?");
      var row = el("div", "jobs-fields");
      row.appendChild(field(p + "provider", "AI", provider));
      var modelBox = field(p + "model", "Model", model);
      modelBox.insertBefore(modelList, model);
      row.appendChild(modelBox);
      var endpointBox = field(p + "endpoint", "Server address", endpoint);
      var folderBox = field(p + "folder", "Folder it may read", folder);
      row.appendChild(endpointBox);
      row.appendChild(folderBox);
      box.appendChild(row);
      box.appendChild(field(p + "prompt", "Prompt", prompt, true));
      var note = el("p", "jobs-subtle jobs-form-note");
      box.appendChild(note);
      box.appendChild(el("p", "jobs-subtle jobs-form-note",
        "The prompt can quote " + PLACEHOLDERS + ". {output} brings in its last 12,000 characters. The answer is this action's output."));

      var OTHER = "__other";
      var listed = null; // names from Ollama, or null when not listed

      function syncList() {
        var useList = provider.value === "ollama" && listed !== null;
        modelList.classList.toggle("d-none", !useList);
        if (!useList) { model.classList.remove("d-none"); return; }
        var name = model.value.trim();
        if (name === "" && listed.length) { name = model.value = listed[0]; }
        var known = listed.indexOf(name) >= 0;
        modelList.value = known ? name : OTHER;
        model.classList.toggle("d-none", known);
      }

      function loadModels() {
        if (provider.value !== "ollama") { syncList(); return; }
        window.__TAURI__.core.invoke("ollama_models", { endpoint: endpoint.value.trim() || null }).then(function (list) {
          var text = list.filter(function (m) { return !m.vision; });
          listed = text.map(function (m) { return m.name; });
          modelList.textContent = "";
          text.forEach(function (m) {
            var o = el("option", "", m.name + " (" + (m.size / 1e9).toFixed(1) + " GB)");
            o.value = m.name;
            modelList.appendChild(o);
          });
          var other = el("option", "", "Other (type a name)");
          other.value = OTHER;
          modelList.appendChild(other);
          syncList();
        }, function () {
          listed = null; // not running: type the name instead
          syncList();
        });
      }

      modelList.addEventListener("change", function () {
        if (modelList.value === OTHER) {
          model.classList.remove("d-none");
          if (listed.indexOf(model.value.trim()) >= 0) { model.value = ""; }
          model.focus();
        } else {
          model.value = modelList.value;
          model.classList.add("d-none");
        }
      });
      provider.addEventListener("change", loadModels);
      endpoint.addEventListener("change", loadModels);

      // Model names belong to one AI (qwen3:1.7b means nothing to Claude
      // Code), so each AI keeps its own while the form is open.
      var models = {};
      var shown = provider.value;
      provider.addEventListener("change", function () {
        models[shown] = model.value;
        shown = provider.value;
        model.value = models[shown] || "";
      });

      function update() {
        var pr = PROVIDERS.filter(function (x) { return x.value === provider.value; })[0];
        model.placeholder = pr.model;
        endpoint.placeholder = ENDPOINTS[pr.value] || "";
        endpointBox.classList.toggle("d-none", !pr.local);
        folderBox.classList.toggle("d-none", !!pr.local);
        note.textContent = pr.local
          ? "The server must already be running; the job never starts it. A local model holds the graphics card, so local AI jobs run one at a time."
          : "Runs the " + pr.label + " command line tool read-only: it may read files in the folder but not change anything. Uses your account's subscription or tokens.";
      }
      provider.addEventListener("change", update);

      return {
        set: function (v) {
          provider.value = v.provider || "ollama";
          model.value = v.model || "";
          models = {};
          shown = provider.value;
          endpoint.value = v.endpoint || "";
          folder.value = v.folder || "";
          prompt.value = v.prompt || "";
          update();
          loadModels();
        },
        read: function () {
          if (!prompt.value.trim()) { throw new Error("the prompt is empty"); }
          var pr = PROVIDERS.filter(function (x) { return x.value === provider.value; })[0];
          var v = { provider: provider.value, model: model.value.trim(), prompt: prompt.value };
          if (pr.local) {
            if (endpoint.value.trim()) { v.endpoint = endpoint.value.trim(); }
          } else if (folder.value.trim()) {
            v.folder = folder.value.trim();
          }
          return v;
        }
      };
    }
  };

  FORMS.notify = function (box, p) {
    var title = input("GBP now {value.text}");
    var text = textarea(3, "It was {previous.text}");
    var urgency = el("select", "form-select");
    [["low", "Low"], ["normal", "Normal"], ["critical", "Critical (stays until closed)"]].forEach(function (u) {
      var o = el("option", "", u[1]);
      o.value = u[0];
      urgency.appendChild(o);
    });
    var sound = el("input", "form-check-input");
    sound.type = "checkbox";
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "title", "Title", title));
    row.appendChild(field(p + "urgency", "Urgency", urgency));
    box.appendChild(row);
    box.appendChild(field(p + "text", "Text", text, true));
    var check = el("div", "form-check form-switch");
    sound.id = p + "sound";
    var soundLabel = el("label", "form-check-label", "Ask the desktop to play its message sound");
    soundLabel.htmlFor = sound.id;
    check.appendChild(sound);
    check.appendChild(soundLabel);
    box.appendChild(check);
    box.appendChild(el("p", "jobs-subtle jobs-form-note",
      "Shows a desktop notification. The title and text can quote " + PLACEHOLDERS +
      ". Whether a sound plays depends on the desktop."));
    return {
      set: function (v) {
        title.value = v.title || "";
        text.value = v.text || "";
        urgency.value = v.urgency || "normal";
        sound.checked = !!v.sound;
      },
      read: function () {
        if (!title.value.trim()) { throw new Error("the title is empty"); }
        var v = { title: title.value, text: text.value, urgency: urgency.value };
        if (sound.checked) { v.sound = true; }
        return v;
      }
    };
  };

  FORMS["switch"] = function (box, p) {
    var section = el("select", "form-select");
    [["trigger", "Trigger"], ["schedule", "Schedule"], ["action", "Action"]].forEach(function (x) {
      var o = el("option", "", x[1]);
      o.value = x[0];
      section.appendChild(o);
    });
    var name = el("select", "form-select");
    var state = el("select", "form-select");
    [["off", "Switch it off"], ["on", "Switch it on"]].forEach(function (x) {
      var o = el("option", "", x[1]);
      o.value = x[0];
      state.appendChild(o);
    });
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "section", "What", section));
    row.appendChild(field(p + "name", "Which", name));
    row.appendChild(field(p + "state", "Do", state));
    box.appendChild(row);
    box.appendChild(el("p", "jobs-subtle jobs-form-note",
      "Switching a trigger on starts it afresh: its next reading only records the value. A schedule switched on makes up no runs from while it was off."));

    function fillNames(keep) {
      var names = window.JobForms.names ? window.JobForms.names(section.value) : [];
      name.textContent = "";
      names.forEach(function (n) {
        var o = el("option", "", n);
        o.value = n;
        name.appendChild(o);
      });
      if (keep && names.indexOf(keep) < 0) {
        var o = el("option", "", keep + " (missing)");
        o.value = keep;
        name.appendChild(o);
      }
      if (keep) { name.value = keep; }
    }
    section.addEventListener("change", function () { fillNames(null); });

    return {
      set: function (v) {
        section.value = v.section || "trigger";
        fillNames(v.name || null);
        state.value = v.enabled ? "on" : "off";
      },
      read: function () {
        if (!name.value) { throw new Error("there is no " + section.value + " to switch yet"); }
        return { section: section.value, name: name.value, enabled: state.value === "on" };
      }
    };
  };

  function options(select, pairs) {
    select.textContent = "";
    pairs.forEach(function (p) {
      var o = el("option", "", p[1]);
      o.value = p[0];
      select.appendChild(o);
    });
  }

  // A list of the entries in one section; keeps a missing name visible.
  function namePicker(select, section, keep) {
    var names = window.JobForms.names ? window.JobForms.names(section) : [];
    options(select, names.map(function (n) { return [n, n]; }));
    if (keep && names.indexOf(keep) < 0) {
      var o = el("option", "", keep + " (missing)");
      o.value = keep;
      select.appendChild(o);
    }
    if (keep) { select.value = keep; }
  }

  function note(box, text) {
    box.appendChild(el("p", "jobs-subtle jobs-form-note", text));
  }

  // Section and name, for set_count and delete.
  function targetForm(box, p, sections, extra) {
    var section = el("select", "form-select");
    options(section, sections);
    var name = el("select", "form-select");
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "section", "What", section));
    row.appendChild(field(p + "name", "Which", name));
    if (extra) { row.appendChild(extra.box); }
    box.appendChild(row);
    section.addEventListener("change", function () { namePicker(name, section.value, null); });
    return {
      set: function (v) {
        section.value = v.section || sections[0][0];
        namePicker(name, section.value, v.name || null);
      },
      read: function () {
        if (!name.value) { throw new Error("there is no " + section.value + " to choose yet"); }
        return { section: section.value, name: name.value };
      }
    };
  }

  FORMS.delay = function (box, p) {
    var schedule = el("select", "form-select");
    var amount = input("2");
    amount.type = "number";
    amount.min = "1";
    var unit = el("select", "form-select");
    options(unit, [["m", "minutes"], ["h", "hours"], ["d", "days"]]);
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "schedule", "Schedule", schedule));
    var pair = el("div", "jobs-pair");
    pair.appendChild(amount);
    pair.appendChild(unit);
    unit.setAttribute("aria-label", "Unit");
    row.appendChild(field(p + "by", "Delay its next run by", pair));
    box.appendChild(row);
    note(box, "Moves the schedule's next run later; the runs after it follow its timetable again.");
    return {
      set: function (v) {
        namePicker(schedule, "schedule", v.schedule || null);
        var m = /^(\d+)\s*([smhd])$/.exec(v.by || "");
        amount.value = m ? m[1] : "1";
        unit.value = m && m[2] !== "s" ? m[2] : "h";
      },
      read: function () {
        if (!schedule.value) { throw new Error("there is no schedule to delay yet"); }
        var n = Number(amount.value);
        if (!(n >= 1)) { throw new Error("the delay needs a number"); }
        return { schedule: schedule.value, by: n + unit.value };
      }
    };
  };

  FORMS.set_count = function (box, p) {
    var count = input("No limit");
    count.type = "number";
    count.min = "1";
    count.max = "999999";
    var target = targetForm(box, p, [["trigger", "Trigger (fire at most)"], ["schedule", "Schedule (run at most)"]],
      { box: field(p + "count", "Set to", count) });
    note(box, "Sets how many more times a trigger may fire or a schedule may run; empty means no limit.");
    return {
      set: function (v) { target.set(v); count.value = v.count ? String(v.count) : ""; },
      read: function () {
        var v = target.read();
        v.count = count.value.trim() ? Number(count.value) : null;
        return v;
      }
    };
  };

  FORMS["delete"] = function (box, p) {
    var target = targetForm(box, p, [["trigger", "Trigger"], ["schedule", "Schedule"], ["action", "Action"]]);
    note(box, "Deletes the entry for good. This action goes with it once it has done its work.");
    return target;
  };

  FORMS.fire = function (box, p) {
    var trigger = el("select", "form-select");
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "trigger", "Trigger", trigger));
    box.appendChild(row);
    note(box, "Reads the trigger's value at once and starts its actions, as its Fire now button does.");
    return {
      set: function (v) { namePicker(trigger, "trigger", v.trigger || null); },
      read: function () {
        if (!trigger.value) { throw new Error("there is no trigger to fire yet"); }
        return { trigger: trigger.value };
      }
    };
  };

  FORMS.log = function (box, p) {
    var level = el("select", "form-select");
    options(level, [["info", "Information"], ["warn", "Warning (Errors page)"], ["error", "Error (Errors page)"]]);
    var text = input("Price now {value.text}");
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "level", "Level", level));
    box.appendChild(row);
    box.appendChild(field(p + "text", "Text", text, true));
    note(box, "Warnings and errors appear on the Errors page; information goes to debug.log when DEBUG_MODE is on. The text can quote " + PLACEHOLDERS + ".");
    return {
      set: function (v) { level.value = v.level || "warn"; text.value = v.text || ""; },
      read: function () {
        if (!text.value.trim()) { throw new Error("the text is empty"); }
        return { level: level.value, text: text.value };
      }
    };
  };

  FORMS.clear_errors = function (box) {
    note(box, "Empties the Errors page list (and errors.jsonl).");
    return { set: function () {}, read: function () { return {}; } };
  };

  FORMS.app = function (box, p) {
    var what = el("select", "form-select");
    options(what, [["show", "Show the window"], ["hide", "Hide the window"], ["quit", "Quit the app"], ["restart", "Restart the app"]]);
    var row = el("div", "jobs-fields");
    row.appendChild(field(p + "do", "Do", what));
    box.appendChild(row);
    note(box, "Hide works as the tray does (Options: Hide to the tray). Quit and restart are never done for a schedule run made up after the app was closed.");
    return {
      set: function (v) { what.value = v["do"] || "show"; },
      read: function () { return { "do": what.value }; }
    };
  };

  // Any other kind: its input as JSON.
  function jsonForm(box, p) {
    var t = textarea(3);
    box.appendChild(field(p + "json", "Input for the job (JSON)", t, true));
    return {
      set: function (v) { t.value = JSON.stringify(v === undefined ? {} : v); },
      read: function () {
        try {
          return JSON.parse(t.value.trim() || "{}");
        } catch (e) {
          throw new Error("the input is not valid JSON (" + e.message + ")");
        }
      }
    };
  }

  // ---- Mounting --------------------------------------------------------------------

  var kinds = [];          // from Rust: [{ name, example, capabilities, resources }]
  var waiting = [];        // mounts made before the kinds arrived
  var loaded = false;

  function exampleOf(name) {
    var k = kinds.filter(function (x) { return x.name === name; })[0];
    try { return k ? JSON.parse(k.example) : {}; } catch (e) { return {}; }
  }

  function mount(container, prefix) {
    var kindSelect = el("select", "form-select");
    var top = el("div", "jobs-fields");
    top.appendChild(field(prefix + "kind", "Kind", kindSelect));
    var body = el("div", "jobs-kind-fields");
    container.textContent = "";
    container.appendChild(top);
    container.appendChild(body);
    var current = null;
    var currentKind = null;

    function show(kind, value) {
      body.textContent = "";
      currentKind = kind;
      current = (FORMS[kind] || jsonForm)(body, prefix + kind + "-");
      current.set(value === undefined ? exampleOf(kind) : value);
    }

    // Rust's kinds (job kinds and built-in ones) in KIND_ORDER, any others after.
    function fillKinds() {
      kindSelect.textContent = "";
      var names = kinds.map(function (k) { return k.name; });
      names.sort(function (a, b) {
        var ia = KIND_ORDER.indexOf(a), ib = KIND_ORDER.indexOf(b);
        return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
      });
      names.forEach(function (name) {
        var o = el("option", "", KIND_LABELS[name] || name);
        o.value = name;
        kindSelect.appendChild(o);
      });
    }

    kindSelect.addEventListener("change", function () { show(kindSelect.value); });

    var api = {
      set: function (kind, value) {
        if (!loaded) { waiting.push(function () { api.set(kind, value); }); return; }
        kindSelect.value = kind || KIND_ORDER[0];
        show(kindSelect.value, value);
      },
      read: function () {
        if (!current) { throw new Error("the job kinds have not loaded yet"); }
        return { kind: currentKind, input: current.read() };
      },
      ready: function () {
        fillKinds();
        if (!current) { show(kindSelect.value); }
      }
    };
    if (loaded) { api.ready(); } else { waiting.push(api.ready); }
    return api;
  }

  window.__TAURI__.core.invoke("jobs_kinds").then(function (list) {
    kinds = list;
    loaded = true;
    var queue = waiting;
    waiting = [];
    queue.forEach(function (fn) { fn(); });
  });

  window.JobForms = { mount: mount, label: function (kind) { return KIND_LABELS[kind] || kind; }, names: null };
})();
