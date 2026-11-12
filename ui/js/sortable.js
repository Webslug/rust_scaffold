/* Drag-and-drop reordering with pointer events (mouse, pen and touch).
   Does not store anything itself: it reports the new order through a
   callback, and app.js saves it to config.json.

   Markup:
     <div class="sortable-list" data-sortable="my-list">
       <div class="sortable-item" data-sortable-id="a">
         <span class="sort-handle" role="button" tabindex="0" aria-label="Reorder A">...</span>
       </div>
       ...
     </div>

   Drag an item by its .sort-handle. Keyboard: focus a handle, then Up or Down
   moves the item, Home and End jump to the ends. Unknown ids in a saved
   order are ignored and items missing from it keep their place at the end,
   so adding or removing items later never breaks a saved order. */
(function () {
  "use strict";

  var status = null;

  function announce(message) {
    if (!status) {
      status = document.createElement("div");
      status.className = "visually-hidden";
      status.setAttribute("role", "status");
      status.setAttribute("aria-live", "polite");
      document.body.appendChild(status);
    }
    status.textContent = message;
  }

  function itemsOf(list) {
    return Array.prototype.slice.call(list.children).filter(function (el) {
      return el.classList.contains("sortable-item");
    });
  }

  function idsOf(list) {
    return itemsOf(list).map(function (el) { return el.getAttribute("data-sortable-id"); });
  }

  function midpoint(el) {
    var r = el.getBoundingClientRect();
    return r.top + r.height / 2;
  }

  function labelOf(item) {
    var title = item.querySelector(".card-header-title, .accordion-button");
    return title ? title.textContent.trim() : item.getAttribute("data-sortable-id");
  }

  /* Puts the items named in `ids` first, in that order; the rest follow in
     their current order. */
  function applyOrder(list, ids) {
    var byId = {};
    var current = itemsOf(list);
    current.forEach(function (el) { byId[el.getAttribute("data-sortable-id")] = el; });
    var ordered = [];
    (ids || []).forEach(function (id) {
      if (byId[id]) { ordered.push(byId[id]); delete byId[id]; }
    });
    current.forEach(function (el) {
      if (byId[el.getAttribute("data-sortable-id")]) { ordered.push(el); }
    });
    ordered.forEach(function (el) { list.appendChild(el); });
  }

  function init(list, onChange) {
    var listId = list.getAttribute("data-sortable");
    var original = idsOf(list);

    function changed() {
      if (onChange) { onChange(listId, idsOf(list)); }
    }

    list.addEventListener("pointerdown", function (e) {
      var handle = e.target.closest(".sort-handle");
      if (!handle || (e.pointerType === "mouse" && e.button !== 0)) { return; }
      var item = handle.closest(".sortable-item");
      if (!item || item.parentNode !== list) { return; }

      e.preventDefault();
      item.classList.add("dragging");
      var pointerId = e.pointerId;
      var startOrder = idsOf(list).join();

      // Listen on the document, not the handle: moving the item in the DOM
      // would release pointer capture and the handle would stop receiving
      // pointer events after the first swap.
      function onMove(ev) {
        if (ev.pointerId !== pointerId) { return; }
        var y = ev.clientY;
        var prev = item.previousElementSibling;
        var next = item.nextElementSibling;
        // One step per event: the layout shifts after each move, so looping
        // here could swap back and forth.
        if (prev && prev.classList.contains("sortable-item") && y < midpoint(prev)) {
          list.insertBefore(item, prev);
        } else if (next && next.classList.contains("sortable-item") && y > midpoint(next)) {
          list.insertBefore(item, next.nextSibling);
        }
      }

      function onEnd(ev) {
        if (ev.pointerId !== pointerId) { return; }
        document.removeEventListener("pointermove", onMove);
        document.removeEventListener("pointerup", onEnd);
        document.removeEventListener("pointercancel", onEnd);
        item.classList.remove("dragging");
        if (idsOf(list).join() !== startOrder) {
          announce(labelOf(item) + " moved to position " + (itemsOf(list).indexOf(item) + 1) + " of " + itemsOf(list).length);
          changed();
        }
      }

      document.addEventListener("pointermove", onMove);
      document.addEventListener("pointerup", onEnd);
      document.addEventListener("pointercancel", onEnd);
    });

    list.addEventListener("keydown", function (e) {
      var handle = e.target.closest(".sort-handle");
      if (!handle) { return; }
      var item = handle.closest(".sortable-item");
      if (!item || item.parentNode !== list) { return; }
      var items = itemsOf(list);
      var index = items.indexOf(item);
      var target = index;
      if (e.key === "ArrowUp") { target = index - 1; }
      else if (e.key === "ArrowDown") { target = index + 1; }
      else if (e.key === "Home") { target = 0; }
      else if (e.key === "End") { target = items.length - 1; }
      else { return; }
      e.preventDefault();
      if (target < 0 || target >= items.length || target === index) { return; }
      var ref = items[target];
      list.insertBefore(item, target < index ? ref : ref.nextSibling);
      handle.focus();
      announce(labelOf(item) + " moved to position " + (target + 1) + " of " + items.length);
      changed();
    });

    return {
      apply: function (ids) { applyOrder(list, ids); },
      reset: function () { applyOrder(list, original); },
      ids: function () { return idsOf(list); }
    };
  }

  window.Sortable = { init: init };
})();
