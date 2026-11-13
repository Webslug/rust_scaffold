/* Help page. Fills in the name, version, rendering mode and settings folder
   from Rust each time the page is opened. The search library line on this
   page is filled in by search.js. */
(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;
  var tab = document.getElementById("tab-help");

  function fill() {
    invoke("app_info").then(function (info) {
      document.getElementById("help-app-name").textContent = info.name;
      document.getElementById("help-version").textContent = info.version;
      document.getElementById("help-config-path").textContent = info.config_path;
      document.getElementById("help-rendering").textContent = info.rendering;
    }).catch(function (err) {
      document.getElementById("help-version").textContent = "Unknown (" + err + ")";
    });
  }

  tab.addEventListener("shown.bs.tab", fill);
})();
