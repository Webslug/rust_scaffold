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

  /* Menu layout, title bar size and accessibility settings become attributes
     and classes on <html>; css/navigation.css and css/a11y-prefs.css do the
     rest. Applied before the window is shown, so nothing flashes. */
  var A11Y_CLASSES = {
    high_contrast: "a11y-contrast",
    large_targets: "a11y-large-targets",
    underline_links: "a11y-underline-links",
    strong_focus: "a11y-strong-focus",
    reduce_motion: "a11y-reduce-motion"
  };
  var menuButton = document.getElementById("btn-menu-toggle");

  function applyMenu() {
    var collapsed = config.menu_collapsed;
    root.setAttribute("data-menu", config.menu_layout === "vertical" ? "vertical" : "horizontal");
    root.classList.toggle("menu-collapsed", collapsed);
    var text = collapsed ? "Expand the menu" : "Contract the menu";
    menuButton.setAttribute("aria-expanded", collapsed ? "false" : "true");
    menuButton.setAttribute("aria-label", text);
    menuButton.title = text;
  }

  function applyPrefs() {
    root.setAttribute("data-titlebar-scale", String(config.titlebar_scale));
    root.setAttribute("data-text-size", String(config.text_size));
    Object.keys(A11Y_CLASSES).forEach(function (key) {
      root.classList.toggle(A11Y_CLASSES[key], !!config[key]);
    });
    applyMenu();
  }

  function setMenuCollapsed(collapsed) {
    if (config.menu_collapsed === collapsed) { return; }
    config.menu_collapsed = collapsed;
    applyMenu();
    save();
  }

  menuButton.addEventListener("click", function () {
    setMenuCollapsed(!config.menu_collapsed);
  });

  // Used by search.js: the search button in the contracted menu expands it.
  window.AppMenu = { setCollapsed: setMenuCollapsed };

  var sorters = {};

  /* The Dashboard example cards (#rust-output, #info-theme, #info-size,
     #btn-app-info) are optional: a project may delete them. Messages then go
     to the warning banner instead. */
  function report(message) {
    if (output) { output.textContent = message; return; }
    warning.textContent = message;
    warning.classList.remove("d-none");
  }

  function renderInfo() {
    var theme = document.getElementById("info-theme");
    var size = document.getElementById("info-size");
    if (theme) { theme.textContent = config.theme; }
    if (size) { size.textContent = config.width + " x " + config.height; }
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
      report("Save failed: " + err);
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

  var infoButton = document.getElementById("btn-app-info");
  if (infoButton) {
    infoButton.addEventListener("click", function () {
      invoke("app_info").then(function (info) {
        report(info.name + " " + info.version + "\nconfig: " + info.config_path);
      }).catch(function (err) {
        report("Error: " + err);
      });
    });
  }

  /* Used by options.js. commit() applies the given changes to the live config
     and saves it. Applying first means a save from elsewhere (a resize, say)
     made meanwhile carries the new values too. If Rust rejects the save, the
     changed keys are put back. Keys Rust knows live at the top level; anything
     else goes into config.extras. */
  function holderOf(key) {
    var builtIn = key !== "extras" && Object.prototype.hasOwnProperty.call(config, key);
    return builtIn ? config : config.extras;
  }

  window.AppConfig = {
    get: function () { return config; },
    canSave: function () { return canSave; },
    commit: function (changes) {
      if (!config) { return Promise.reject("config is not loaded"); }
      if (!canSave) { return Promise.reject("saving is disabled because config.json could not be loaded"); }
      var keys = Object.keys(changes);
      var previous = {};
      keys.forEach(function (key) {
        var holder = holderOf(key);
        previous[key] = { had: Object.prototype.hasOwnProperty.call(holder, key), value: holder[key] };
        holder[key] = changes[key];
      });
      function applyLive() {
        config.theme = window.AppTheme.apply(config.theme);
        root.classList.toggle("square-corners", !config.rounded_corners);
        applyTitlebar();
        applyPrefs();
        renderInfo();
      }
      return invoke("save_config", { config: config }).then(applyLive, function (err) {
        keys.forEach(function (key) {
          var holder = holderOf(key);
          if (previous[key].had) { holder[key] = previous[key].value; }
          else { delete holder[key]; }
        });
        throw err;
      });
    }
  };

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
    applyPrefs();
    renderInfo();
    setupSortables();
    setupAccordions();
    if (WARNINGS[reply.status]) {
      warning.textContent = WARNINGS[reply.status];
      warning.classList.remove("d-none");
    }
  }).catch(function (err) {
    report("Could not load config: " + err);
  }).then(function () {
    // The window starts hidden so there is no flash of the default theme.
    // With start_minimised (and a working tray) it simply stays hidden.
    return startHidden ? null : appWindow.show();
  });
})();
