/* Window controls for the frameless window: minimise, maximise/restore and
   close, plus a "maximized" class on <html> so the CSS can drop the rounded
   corners while the window fills the screen. Dragging is handled natively by
   the data-tauri-drag-region attribute in index.html. */
(function () {
  "use strict";

  var appWindow = window.__TAURI__.window.getCurrentWindow();
  var root = document.documentElement;

  function syncMaximized() {
    appWindow.isMaximized().then(function (maximized) {
      root.classList.toggle("maximized", maximized);
      var icon = document.querySelector("#btn-maximize .bi");
      if (icon) {
        icon.classList.toggle("bi-square", !maximized);
        icon.classList.toggle("bi-fullscreen-exit", maximized);
      }
    });
  }

  document.getElementById("btn-minimize").addEventListener("click", function () {
    appWindow.minimize();
  });
  document.getElementById("btn-maximize").addEventListener("click", function () {
    appWindow.toggleMaximize();
  });
  document.getElementById("btn-close").addEventListener("click", function () {
    appWindow.close();
  });

  window.addEventListener("resize", syncMaximized);
  syncMaximized();
})();
