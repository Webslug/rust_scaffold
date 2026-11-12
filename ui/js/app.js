/* Application logic. Loads config.json through Rust, applies the saved theme,
   saves theme and window size back, and shows the window once it is ready. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;
  var appWindow = window.__TAURI__.window.getCurrentWindow();

  var MIN_SAVE_WIDTH = 300;   // below this the window is minimised or collapsing
  var RESIZE_DEBOUNCE_MS = 500;

  var WARNINGS = {
    too_large: "config.json is larger than 5 MB, so it was not loaded. Defaults are in use and saving is disabled.",
    invalid: "config.json could not be parsed or holds invalid values. Defaults are in use and saving is disabled so the file is not overwritten.",
    unreadable: "config.json could not be read or created. Defaults are in use and saving is disabled."
  };

  var output = document.getElementById("rust-output");
  var warning = document.getElementById("config-warning");

  var config = null;        // last known config
  var canSave = false;
  var resizeTimer = null;
  var startHidden = false;

  var root = document.documentElement;
  var titleButton = document.getElementById("btn-title");

  function applyTitlebar() {
    root.classList.toggle("titlebar-hidden", !config.show_titlebar);
    titleButton.setAttribute("aria-pressed", config.show_titlebar ? "true" : "false");
  }

  function toggleTitlebar() {
    config.show_titlebar = !config.show_titlebar;
    applyTitlebar();
    save();
  }

  var sorters = {};

  function renderInfo() {
    document.getElementById("info-theme").textContent = config.theme;
    document.getElementById("info-size").textContent = config.width + " x " + config.height;
  }

  function setupSortables() {
    document.querySelectorAll("[data-sortable]").forEach(function (list) {
      var key = list.getAttribute("data-sortable");
      var sorter = window.Sortable.init(list, function (listId, ids) {
        config.order[listId] = ids;
        save();
      });
      if (config.order[key]) { sorter.apply(config.order[key]); }
      sorters[key] = sorter;
    });
  }

  document.querySelectorAll("[data-reset-sortable]").forEach(function (button) {
    button.addEventListener("click", function () {
      var key = button.getAttribute("data-reset-sortable");
      if (!sorters[key]) { return; }
      sorters[key].reset();
      delete config.order[key];
      save();
    });
  });

  /* Accordion panels start expanded. Ids in config.collapsed are collapsed
     before the window is shown, so there is no animation or flash. Opening
     or closing a panel adds or removes only its own id, so ids this page
     does not know about are left alone. */
  function setupAccordions() {
    document.querySelectorAll("[data-panel-id]").forEach(function (panel) {
      var id = panel.getAttribute("data-panel-id");
      if (config.collapsed.indexOf(id) === -1) { return; }
      panel.classList.remove("show");
      var button = document.querySelector('[data-bs-target="#' + panel.id + '"]');
      if (button) {
        button.classList.add("collapsed");
        button.setAttribute("aria-expanded", "false");
      }
    });
  }

  function setCollapsed(id, collapsed) {
    var at = config.collapsed.indexOf(id);
    if (collapsed && at === -1) { config.collapsed.push(id); }
    else if (!collapsed && at !== -1) { config.collapsed.splice(at, 1); }
    else { return; }
    save();
  }

  document.addEventListener("hidden.bs.collapse", function (e) {
    var id = e.target.getAttribute("data-panel-id");
    if (id) { setCollapsed(id, true); }
  });
  document.addEventListener("shown.bs.collapse", function (e) {
    var id = e.target.getAttribute("data-panel-id");
    if (id) { setCollapsed(id, false); }
  });

  function save() {
    if (!canSave) { return Promise.resolve(); }
    return invoke("save_config", { config: config }).catch(function (err) {
      output.textContent = "Save failed: " + err;
    });
  }

  function saveSize() {
    appWindow.isMaximized().then(function (maximized) {
      var w = window.innerWidth;
      var h = window.innerHeight;
      if (maximized || w < MIN_SAVE_WIDTH) { return; }
      if (w === config.width && h === config.height) { return; }
      config.width = w;
      config.height = h;
      renderInfo();
      save();
    });
  }

  document.getElementById("btn-app-info").addEventListener("click", function () {
    invoke("app_info").then(function (info) {
      output.textContent = info.name + " " + info.version + "\nconfig: " + info.config_path;
    }).catch(function (err) {
      output.textContent = "Error: " + err;
    });
  });

  titleButton.addEventListener("click", toggleTitlebar);
  document.getElementById("btn-title-reveal").addEventListener("click", toggleTitlebar);

  window.AppTheme.onChange(function (id) {
    config.theme = id;
    renderInfo();
    save();
  });

  window.addEventListener("resize", function () {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(saveSize, RESIZE_DEBOUNCE_MS);
  });

  invoke("get_config").then(function (reply) {
    config = reply.config;
    canSave = reply.status === "ok" || reply.status === "created";
    startHidden = reply.start_hidden;
    config.theme = window.AppTheme.apply(config.theme);
    root.classList.toggle("square-corners", !config.rounded_corners);
    applyTitlebar();
    renderInfo();
    setupSortables();
    setupAccordions();
    if (WARNINGS[reply.status]) {
      warning.textContent = WARNINGS[reply.status];
      warning.classList.remove("d-none");
    }
  }).catch(function (err) {
    output.textContent = "Could not load config: " + err;
  }).then(function () {
    // The window starts hidden so there is no flash of the default theme.
    // With start_minimised (and a working tray) it simply stays hidden.
    return startHidden ? null : appWindow.show();
  });
})();
