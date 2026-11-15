/* Input fields for each job kind, shared by the Queue view ("Run a job
   now") and the schedule editor (js/schedules.js). The kinds themselves are
   defined in src-tauri/src/job_kinds.rs; Rust checks every input again
   before a job is queued or a schedule saved, so these forms only help to
   fill it in.

   window.JobForms.mount(container, prefix) returns an object with
   set(kind, input) and read() -> { kind, input } (read throws an Error with
   a message for people when a field cannot be read). A kind without a form
   here gets a plain JSON box, so new kinds work before they get a form. */
(function () {
  "use strict";

  var PROVIDERS = [
    { value: "ollama", label: "Ollama (local)", model: "qwen3:1.7b", local: true },
    { value: "llama_cpp", label: "llama.cpp (local)", model: "optional", local: true },
    { value: "claude_code", label: "Claude Code", model: "optional, for example sonnet" },
    { value: "codex", label: "Codex", model: "optional, the tool's default" },
    { value: "grok_build", label: "GrokBuild", model: "optional, the tool's default" }
  ];

  var KIND_LABELS = { command: "Command (bash)", ai: "AI prompt" };

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
        "When a rule starts this job, the job before it is in $PREVIOUS_OUTPUT (its output; the whole of it in the file $PREVIOUS_OUTPUT_FILE), " +
        "$PREVIOUS_LABEL, $PREVIOUS_STATE and $PREVIOUS_SCHEDULE. Use them in quotes, for example \"$PREVIOUS_OUTPUT\"."));
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
        "When a rule starts this job, {previous_output} in the prompt becomes the output of the job before it (its last 12,000 characters). " +
        "{previous_label}, {previous_state} and {previous_schedule} work too."));

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
    top.appendChild(field(prefix + "kind", "Job", kindSelect));
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

    function fillKinds() {
      kindSelect.textContent = "";
      kinds.forEach(function (k) {
        var o = el("option", "", KIND_LABELS[k.name] || k.name);
        o.value = k.name;
        kindSelect.appendChild(o);
      });
    }

    kindSelect.addEventListener("change", function () { show(kindSelect.value); });

    var api = {
      set: function (kind, value) {
        if (!loaded) { waiting.push(function () { api.set(kind, value); }); return; }
        kindSelect.value = kind || (kinds[0] && kinds[0].name);
        show(kindSelect.value, value);
      },
      read: function () {
        if (!current) { throw new Error("the job kinds have not loaded yet"); }
        return { kind: currentKind, input: current.read() };
      },
      ready: function () {
        fillKinds();
        if (!current && kinds.length) { show(kinds[0].name); }
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

  window.JobForms = { mount: mount, label: function (kind) { return KIND_LABELS[kind] || kind; } };
})();
