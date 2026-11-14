/* Theme list and switching. The chosen theme is stored in config.json by
   app.js, not in localStorage. Sets data-theme on <html>; the colours and
   window gradients are tokens in css/variables.css. */
(function () {
  "use strict";

  var THEMES = [
    { id: "aero-blue", label: "Aero Blue" },
    { id: "baby-pink", label: "Baby Pink" },
    { id: "materia", label: "Materia" },
    { id: "sketchy", label: "Sketchy" },
    { id: "slate", label: "Slate" },
    { id: "dark-mode", label: "Dark Mode" },
    { id: "lilac", label: "Lilac" },
    { id: "regal", label: "Regal" },
    { id: "racing-green", label: "Racing Green" },
    { id: "orange-black", label: "Orange + Black" },
    { id: "regal-blue", label: "Regal Blue" },
    { id: "rose", label: "Rose" },
    { id: "vapor", label: "Vapor" },
    { id: "quartz", label: "Quartz" }
  ];
  var DEFAULT_THEME = "racing-green";

  var root = document.documentElement;
  var menu = document.getElementById("theme-menu");
  var label = document.getElementById("theme-label");
  var onChange = null;

  function find(id) {
    return THEMES.filter(function (t) { return t.id === id; })[0];
  }

  function apply(id) {
    var theme = find(id) || find(DEFAULT_THEME);
    root.setAttribute("data-theme", theme.id);
    label.textContent = theme.label;
    menu.querySelectorAll(".dropdown-item").forEach(function (item) {
      var active = item.getAttribute("data-theme-value") === theme.id;
      item.classList.toggle("active", active);
      if (active) { item.setAttribute("aria-current", "true"); }
      else { item.removeAttribute("aria-current"); }
    });
    return theme.id;
  }

  THEMES.forEach(function (theme) {
    var li = document.createElement("li");
    var a = document.createElement("a");
    a.className = "dropdown-item";
    a.href = "#";
    a.textContent = theme.label;
    a.setAttribute("data-theme-value", theme.id);
    a.addEventListener("click", function (e) {
      e.preventDefault();
      apply(theme.id);
      if (onChange) { onChange(theme.id); }
    });
    li.appendChild(a);
    menu.appendChild(li);
  });

  window.AppTheme = {
    apply: apply,
    list: function () { return THEMES.slice(); },
    onChange: function (fn) { onChange = fn; }
  };
})();
