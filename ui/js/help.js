/* Help page. Fills in the name, version, rendering mode, settings folder and build details
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
      fillBuild(info.build);
    }).catch(function (err) {
      document.getElementById("help-version").textContent = "Unknown (" + err + ")";
    });
  }

  // Build details recorded by build.rs. The time is shown in local time.
  function fillBuild(build) {
    if (!build) {
      return;
    }
    var when = build.unix_time ? new Date(build.unix_time * 1000).toLocaleString("en-GB", {
      day: "numeric", month: "long", year: "numeric", hour: "2-digit", minute: "2-digit"
    }) : "Unknown";
    document.getElementById("help-build-time").textContent = when;
    document.getElementById("help-build-rustc").textContent = build.rustc;
    document.getElementById("help-build-cargo").textContent = build.cargo;
    document.getElementById("help-build-profile").textContent = build.profile;

    var crates = document.getElementById("help-build-crates");
    crates.textContent = "";
    build.crates.forEach(function (item) {
      var line = document.createElement("span");
      line.className = "d-block";
      line.textContent = item;
      crates.appendChild(line);
    });
    var total = document.createElement("span");
    total.className = "d-block text-muted";
    total.textContent = build.crate_count + " crates in total, including dependencies";
    crates.appendChild(total);
  }

  tab.addEventListener("shown.bs.tab", fill);
})();
