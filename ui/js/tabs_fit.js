/* Compact tab rows. For every list marked data-tabs-fit: when the tab labels
   would wrap onto a second row, hide the labels and show icons only, with a
   Bootstrap tooltip naming each tab. If even the icons do not fit, the row
   scrolls sideways (see .tabs-icons in window.css). Works for any number of
   tabs. Labels must be in <span class="tab-label">; they stay readable to
   screen readers when hidden. */
(function () {
  "use strict";

  var COMPACT = "tabs-icons";

  function wraps(list) {
    var items = list.children;
    if (items.length < 2) { return false; }
    return items[items.length - 1].offsetTop > items[0].offsetTop;
  }

  function setup(list) {
    var tips = [];
    var compact = false;
    var pending = false;

    list.querySelectorAll(".nav-link").forEach(function (link) {
      var label = link.querySelector(".tab-label");
      if (!label) { return; }
      var tip = new bootstrap.Tooltip(link, {
        title: label.textContent.trim(),
        placement: "bottom",
        trigger: "hover focus"
      });
      tip.disable();
      // A clicked tab keeps focus; hide its tooltip so it does not linger.
      link.addEventListener("click", function () { tip.hide(); });
      tips.push(tip);
    });

    function setCompact(on) {
      if (on === compact) { return; }
      compact = on;
      list.classList.toggle(COMPACT, on);
      tips.forEach(function (tip) {
        if (on) { tip.enable(); }
        else { tip.hide(); tip.disable(); }
      });
    }

    /* Measures with full labels; both class changes happen in one frame
       before any paint, so nothing flickers. */
    function check() {
      pending = false;
      list.classList.remove(COMPACT);
      var tooWide = wraps(list);
      list.classList.toggle(COMPACT, compact);
      setCompact(tooWide);
    }

    function schedule() {
      if (pending) { return; }
      pending = true;
      requestAnimationFrame(check);
    }

    new ResizeObserver(schedule).observe(list);
    // Theme fonts differ in width (Sketchy is much wider).
    new MutationObserver(schedule).observe(document.documentElement, {
      attributes: true, attributeFilter: ["data-theme"]
    });
    if (document.fonts) { document.fonts.ready.then(schedule); }

    // Keep the selected tab visible when the row scrolls sideways.
    list.addEventListener("shown.bs.tab", function (e) {
      if (compact) { e.target.scrollIntoView({ block: "nearest", inline: "nearest" }); }
    });
    check();
  }

  document.querySelectorAll("[data-tabs-fit]").forEach(setup);
})();
