/* Search each mock table independently. Search terms can match different
   columns in the same row; no Rust call or config write is needed. */
(function () {
  "use strict";

  document.querySelectorAll("input[data-table-search]").forEach(function (input) {
    var table = document.getElementById(input.getAttribute("data-table-search"));
    var status = document.getElementById(input.getAttribute("data-status-id"));
    if (!table || !table.tBodies.length || !status) { return; }

    var rows = Array.prototype.slice.call(table.tBodies[0].rows).filter(function (row) {
      return !row.classList.contains("table-no-results");
    });
    var emptyRow = table.tBodies[0].querySelector(".table-no-results");
    var searchable = rows.map(function (row) {
      return row.textContent.replace(/\s+/g, " ").toLowerCase();
    });

    function filterRows() {
      var terms = input.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
      var visible = 0;
      rows.forEach(function (row, index) {
        var matches = terms.every(function (term) { return searchable[index].includes(term); });
        row.hidden = !matches;
        if (matches) { visible += 1; }
      });
      if (emptyRow) { emptyRow.hidden = visible !== 0; }
      status.textContent = visible + " of " + rows.length + (rows.length === 1 ? " entry" : " entries");
    }

    input.addEventListener("input", filterRows);
    filterRows();
  });
})();
