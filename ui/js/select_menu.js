/* Themed lists for select boxes. WebKitGTK draws the list a <select> opens
   with GTK, outside the page, so CSS cannot give it the theme's colours.
   This script opens a list of its own instead, drawn in the page with the
   theme's tokens (css/select_menu.css), for every select.form-select,
   including ones added later.

   The real <select> keeps the value: choosing an option sets it and fires
   its usual "change" event, so no other script changes. Mouse press, Space,
   Enter and Alt+Down open the list; in the list Up, Down, Home, End and the
   first letter move, Enter or Space choose, Escape or Tab close. Up and
   Down on a closed select still change it directly, as before. Disabled
   selects and options are respected. */
(function () {
  "use strict";

  var menu = null;     // the open list
  var owner = null;    // the select it belongs to
  var active = -1;     // index of the highlighted option

  function usable(el) {
    return el && el.tagName === "SELECT" && el.classList.contains("form-select") &&
      !el.disabled && !el.multiple && !(el.size > 1);
  }

  function close(refocus) {
    if (!menu) { return; }
    menu.remove();
    if (owner) {
      owner.setAttribute("aria-expanded", "false");
      if (refocus) { owner.focus(); }
    }
    menu = null;
    owner = null;
    active = -1;
  }

  function items() {
    return menu ? Array.prototype.slice.call(menu.querySelectorAll(".select-menu-option")) : [];
  }

  function highlight(index) {
    var list = items();
    if (!list.length) { return; }
    index = Math.max(0, Math.min(list.length - 1, index));
    // Skip disabled options in the direction of travel.
    var step = index >= active ? 1 : -1;
    while (list[index] && list[index].classList.contains("disabled")) { index += step; }
    if (!list[index]) { return; }
    list.forEach(function (el, i) { el.classList.toggle("active", i === index); });
    active = index;
    list[index].scrollIntoView({ block: "nearest" });
  }

  function choose(index) {
    var el = items()[index];
    if (!el || el.classList.contains("disabled")) { return; }
    var select = owner;
    var value = el.dataset.value;
    close(true);
    if (select.value !== value) {
      select.value = value;
      select.dispatchEvent(new Event("input", { bubbles: true }));
      select.dispatchEvent(new Event("change", { bubbles: true }));
    }
  }

  // Places the list under the select, or above it when there is more room.
  function place() {
    var r = owner.getBoundingClientRect();
    menu.style.minWidth = r.width + "px";
    menu.style.left = Math.max(4, Math.min(r.left, window.innerWidth - menu.offsetWidth - 4)) + "px";
    var below = window.innerHeight - r.bottom;
    if (below < menu.offsetHeight + 8 && r.top > below) {
      menu.style.top = Math.max(4, r.top - menu.offsetHeight - 2) + "px";
    } else {
      menu.style.top = (r.bottom + 2) + "px";
    }
  }

  function open(select) {
    close(false);
    owner = select;
    menu = document.createElement("div");
    menu.className = "select-menu";
    menu.setAttribute("role", "listbox");
    var label = select.getAttribute("aria-label") ||
      (select.id && document.querySelector('label[for="' + select.id + '"]') ? document.querySelector('label[for="' + select.id + '"]').textContent : "");
    if (label) { menu.setAttribute("aria-label", label); }
    Array.prototype.forEach.call(select.options, function (opt, i) {
      var el = document.createElement("div");
      el.className = "select-menu-option" + (opt.disabled ? " disabled" : "") + (opt.selected ? " selected" : "");
      el.setAttribute("role", "option");
      el.setAttribute("aria-selected", opt.selected ? "true" : "false");
      el.dataset.value = opt.value;
      el.dataset.index = String(i);
      el.textContent = opt.textContent;
      menu.appendChild(el);
    });
    document.body.appendChild(menu);
    select.setAttribute("aria-expanded", "true");
    place();
    active = -1;
    highlight(select.selectedIndex < 0 ? 0 : select.selectedIndex);
  }

  // Mouse: open instead of the GTK list.
  document.addEventListener("mousedown", function (e) {
    if (menu && menu.contains(e.target)) {
      e.preventDefault(); // keep the focus on the select
      var option = e.target.closest(".select-menu-option");
      if (option) { choose(Number(option.dataset.index)); }
      return;
    }
    var select = e.target.closest("select");
    if (usable(select)) {
      e.preventDefault();
      select.focus();
      if (owner === select) { close(true); } else { open(select); }
      return;
    }
    close(false);
  }, true);

  document.addEventListener("mousemove", function (e) {
    if (!menu) { return; }
    var option = e.target.closest(".select-menu-option");
    if (option && menu.contains(option)) {
      var i = Number(option.dataset.index);
      if (i !== active && !option.classList.contains("disabled")) {
        items().forEach(function (el, n) { el.classList.toggle("active", n === i); });
        active = i;
      }
    }
  });

  document.addEventListener("keydown", function (e) {
    if (menu && document.activeElement === owner) {
      var list = items();
      if (e.key === "ArrowDown") { highlight(active + 1); }
      else if (e.key === "ArrowUp") { highlight(active - 1); }
      else if (e.key === "Home") { active = -1; highlight(0); }
      else if (e.key === "End") { active = list.length; highlight(list.length - 1); }
      else if (e.key === "Enter" || e.key === " ") { choose(active); }
      else if (e.key === "Escape") { close(true); }
      else if (e.key === "Tab") { close(false); return; }
      else if (e.key.length === 1) {
        var letter = e.key.toLowerCase();
        var from = active + 1;
        for (var n = 0; n < list.length; n++) {
          var i = (from + n) % list.length;
          if (list[i].textContent.trim().toLowerCase().indexOf(letter) === 0 && !list[i].classList.contains("disabled")) {
            active = i - 1;
            highlight(i);
            break;
          }
        }
      } else { return; }
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    var select = e.target;
    if (usable(select) && (e.key === "Enter" || e.key === " " || (e.altKey && e.key === "ArrowDown"))) {
      e.preventDefault();
      e.stopPropagation();
      open(select);
    }
  }, true);

  // The list is placed for the select's position; anything that moves the
  // select closes it.
  window.addEventListener("resize", function () { close(false); });
  document.addEventListener("scroll", function (e) {
    if (menu && !menu.contains(e.target)) { close(false); }
  }, true);
  document.addEventListener("focusin", function (e) {
    if (menu && e.target !== owner && !menu.contains(e.target)) { close(false); }
  });
})();
