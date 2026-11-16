/* A date and time picker in the theme's colours, for text boxes marked
   data-picker="datetime" (YYYY-MM-DD HH:MM) or data-picker="time" (HH:MM),
   the formats jobs.json uses. Clicking or focusing the box opens it below;
   choosing a day, an hour and a minute writes the box and fires "input" and
   "change". The box can still be typed into; Escape or a click elsewhere
   closes the picker. No library: Bootstrap has no date picker. Styles in
   css/datetime_picker.css. */
(function () {
  "use strict";

  var DAYS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
  var MONTHS = ["January", "February", "March", "April", "May", "June", "July",
    "August", "September", "October", "November", "December"];

  var box = null;      // the open picker
  var input = null;    // the text box it fills
  var shown = null;    // first day of the month shown
  var chosen = null;   // { y, m, d, hh, mm }
  var returning = null; // the box getting focus back after Done or Escape

  function pad(n) { return (n < 10 ? "0" : "") + n; }

  function el(tag, className, text) {
    var e = document.createElement(tag);
    if (className) { e.className = className; }
    if (text !== undefined) { e.textContent = text; }
    return e;
  }

  // What the box holds now, or now.
  function parse(kind, text) {
    var now = new Date();
    var v = { y: now.getFullYear(), m: now.getMonth(), d: now.getDate(), hh: now.getHours(), mm: 0 };
    var m = kind === "time"
      ? /^\s*(\d{1,2}):(\d{2})\s*$/.exec(text)
      : /^\s*(\d{4})-(\d{2})-(\d{2})(?:\s+(\d{1,2}):(\d{2}))?\s*$/.exec(text);
    if (m && kind === "time") { v.hh = Number(m[1]); v.mm = Number(m[2]); }
    if (m && kind !== "time") {
      v.y = Number(m[1]); v.m = Number(m[2]) - 1; v.d = Number(m[3]);
      if (m[4] !== undefined) { v.hh = Number(m[4]); v.mm = Number(m[5]); }
    }
    v.hh = Math.min(23, Math.max(0, v.hh));
    v.mm = Math.min(59, Math.max(0, v.mm));
    return v;
  }

  function write() {
    var kind = input.dataset.picker;
    var time = pad(chosen.hh) + ":" + pad(chosen.mm);
    input.value = kind === "time" ? time : chosen.y + "-" + pad(chosen.m + 1) + "-" + pad(chosen.d) + " " + time;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function close() {
    if (box) { box.remove(); }
    box = null;
    input = null;
  }

  function numberSelect(label, count, value, step) {
    var s = el("select", "form-select form-select-sm");
    s.setAttribute("aria-label", label);
    for (var i = 0; i < count; i += step) {
      var o = el("option", "", pad(i));
      o.value = String(i);
      s.appendChild(o);
    }
    if (value % step) {
      var extra = el("option", "", pad(value));
      extra.value = String(value);
      s.appendChild(extra);
    }
    s.value = String(value);
    return s;
  }

  function drawCalendar(grid, title) {
    grid.textContent = "";
    title.textContent = MONTHS[shown.getMonth()] + " " + shown.getFullYear();
    DAYS.forEach(function (d) { grid.appendChild(el("span", "dtp-weekday", d)); });
    var first = (shown.getDay() + 6) % 7; // Monday first
    for (var i = 0; i < first; i++) { grid.appendChild(el("span", "dtp-blank")); }
    var days = new Date(shown.getFullYear(), shown.getMonth() + 1, 0).getDate();
    var today = new Date();
    for (var d = 1; d <= days; d++) {
      var b = el("button", "dtp-day", String(d));
      b.type = "button";
      b.dataset.day = String(d);
      var isChosen = chosen.y === shown.getFullYear() && chosen.m === shown.getMonth() && chosen.d === d;
      var isToday = today.getFullYear() === shown.getFullYear() && today.getMonth() === shown.getMonth() && today.getDate() === d;
      if (isChosen) { b.classList.add("chosen"); b.setAttribute("aria-pressed", "true"); }
      if (isToday) { b.classList.add("today"); }
      grid.appendChild(b);
    }
  }

  function open(target) {
    close();
    input = target;
    var kind = input.dataset.picker;
    chosen = parse(kind, input.value);
    shown = new Date(chosen.y, chosen.m, 1);
    box = el("div", "dtp");
    box.setAttribute("role", "dialog");
    box.setAttribute("aria-label", kind === "time" ? "Choose a time" : "Choose a date and time");

    if (kind !== "time") {
      var head = el("div", "dtp-head");
      var prev = el("button", "btn btn-secondary btn-sm");
      prev.type = "button";
      prev.setAttribute("aria-label", "Previous month");
      prev.appendChild(el("i", "bi bi-chevron-left"));
      var next = el("button", "btn btn-secondary btn-sm");
      next.type = "button";
      next.setAttribute("aria-label", "Next month");
      next.appendChild(el("i", "bi bi-chevron-right"));
      var title = el("span", "dtp-title");
      head.appendChild(prev);
      head.appendChild(title);
      head.appendChild(next);
      var grid = el("div", "dtp-grid");
      box.appendChild(head);
      box.appendChild(grid);
      drawCalendar(grid, title);
      prev.addEventListener("click", function () { shown = new Date(shown.getFullYear(), shown.getMonth() - 1, 1); drawCalendar(grid, title); });
      next.addEventListener("click", function () { shown = new Date(shown.getFullYear(), shown.getMonth() + 1, 1); drawCalendar(grid, title); });
      grid.addEventListener("click", function (e) {
        var b = e.target.closest(".dtp-day");
        if (!b) { return; }
        chosen.y = shown.getFullYear();
        chosen.m = shown.getMonth();
        chosen.d = Number(b.dataset.day);
        drawCalendar(grid, title);
        write();
      });
    }

    var row = el("div", "dtp-time");
    row.appendChild(el("i", "bi bi-clock"));
    var hours = numberSelect("Hour", 24, chosen.hh, 1);
    var minutes = numberSelect("Minute", 60, chosen.mm, 5);
    row.appendChild(hours);
    row.appendChild(el("span", "dtp-colon", ":"));
    row.appendChild(minutes);
    hours.addEventListener("change", function () { chosen.hh = Number(hours.value); write(); });
    minutes.addEventListener("change", function () { chosen.mm = Number(minutes.value); write(); });
    box.appendChild(row);

    var foot = el("div", "dtp-foot");
    var now = el("button", "btn btn-secondary btn-sm", "Now");
    now.type = "button";
    var done = el("button", "btn btn-primary btn-sm", "Done");
    done.type = "button";
    foot.appendChild(now);
    foot.appendChild(done);
    box.appendChild(foot);
    now.addEventListener("click", function () {
      var t = new Date();
      chosen = { y: t.getFullYear(), m: t.getMonth(), d: t.getDate(), hh: t.getHours(), mm: t.getMinutes() };
      write();
      open(input);
    });
    done.addEventListener("click", function () {
      var target = input;
      write();
      close();
      returning = target;
      target.focus();
    });

    document.body.appendChild(box);
    var r = input.getBoundingClientRect();
    var below = window.innerHeight - r.bottom;
    box.style.left = Math.max(4, Math.min(r.left, window.innerWidth - box.offsetWidth - 4)) + "px";
    box.style.top = (below < box.offsetHeight + 8 && r.top > below ? Math.max(4, r.top - box.offsetHeight - 2) : r.bottom + 2) + "px";
  }

  document.addEventListener("focusin", function (e) {
    var t = e.target;
    if (t === returning) { returning = null; return; }
    returning = null;
    if (t.matches && t.matches("input[data-picker]") && t !== input) { open(t); }
  });

  document.addEventListener("mousedown", function (e) {
    if (!box) { return; }
    // Clicks inside the picker, on its box or in a themed select list it
    // opened keep it open.
    if (box.contains(e.target) || e.target === input || e.target.closest(".select-menu")) { return; }
    if (e.target.matches && e.target.matches("input[data-picker]")) { return; } // focusin opens it
    close();
  }, true);

  document.addEventListener("keydown", function (e) {
    if (box && e.key === "Escape") {
      e.preventDefault();
      var target = input;
      close();
      returning = target;
      target.focus();
    }
  });

  window.addEventListener("resize", close);
})();
