/* Fictional chart examples. Each card owns its own search and filter state. */
(function () {
  "use strict";

  var MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun"];
  var DATA = {
    sales: {
      labels: MONTHS,
      prefix: "$",
      suffix: "k",
      series: [
        { name: "Sales", values: [42, 48, 51, 47, 58, 64] },
        { name: "Costs", values: [31, 33, 36, 35, 38, 41] }
      ]
    },
    signups: {
      labels: MONTHS,
      series: [
        { name: "Direct", values: [120, 132, 129, 141, 153, 162] },
        { name: "Search", values: [86, 91, 105, 109, 122, 137] },
        { name: "Social", values: [34, 46, 45, 53, 67, 75] }
      ]
    },
    retention: {
      centerLabel: "Customers",
      items: [
        { name: "Returning", value: 620 },
        { name: "New", value: 250 },
        { name: "Lapsed", value: 130 }
      ]
    },
    segments: {
      suffix: "%",
      items: [
        { name: "Retail", value: 45 },
        { name: "Business", value: 30 },
        { name: "Student", value: 15 },
        { name: "Other", value: 10 }
      ]
    }
  };

  function filtered(data, query, mode) {
    if (data.labels) {
      var indices = data.labels.map(function (_, i) { return i; }).filter(function (i) {
        return (mode === "all" || (mode === "first" ? i < 3 : i >= 3)) &&
          data.labels[i].toLowerCase().includes(query);
      });
      return {
        labels: indices.map(function (i) { return data.labels[i]; }),
        prefix: data.prefix || "",
        suffix: data.suffix || "",
        series: data.series.map(function (series) {
          return { name: series.name, values: indices.map(function (i) { return series.values[i]; }) };
        })
      };
    }
    var ranked = data.items.slice().sort(function (a, b) { return b.value - a.value; });
    var selected = mode === "largest" ? ranked.slice(0, 2) :
      mode === "smallest" ? ranked.slice(-2) : data.items;
    return {
      centerLabel: data.centerLabel || "Total",
      suffix: data.suffix || "",
      items: data.items.map(function (item, index) {
        return { name: item.name, value: item.value, colorSlot: index + 1 };
      }).filter(function (item) {
        return selected.some(function (candidate) { return candidate.name === item.name; }) &&
          item.name.toLowerCase().includes(query);
      })
    };
  }

  function count(config) {
    return config.labels ? config.labels.length : config.items.length;
  }

  document.addEventListener("DOMContentLoaded", function () {
    document.querySelectorAll("[data-chart-example]").forEach(function (card) {
      var data = DATA[card.getAttribute("data-chart-example")];
      var chart = card.querySelector("[data-chart]");
      var search = card.querySelector('input[type="search"]');
      var select = card.querySelector("select");
      var status = card.querySelector(".chart-status");
      var table = card.querySelector(".chart-table-wrap");
      var toggle = card.querySelector("[data-chart-table-toggle]");
      var total = data.labels ? data.labels.length : data.items.length;

      function render(initial) {
        var config = filtered(data, search.value.trim().toLowerCase(), select.value);
        var matches = count(config);
        status.textContent = matches + " of " + total + (data.labels ? " months" : " groups") +
          " shown" + (data.items && matches ? ". Shares use visible groups." : ".");
        toggle.disabled = matches === 0;
        if (!matches) {
          chart.hidden = true;
          table.hidden = true;
          status.textContent += " No matching data.";
          return;
        }
        if (initial) { chart.setAttribute("data-config", JSON.stringify(config)); }
        else { window.Charts.update(chart, config); }
        var tableShown = toggle.getAttribute("aria-pressed") === "true";
        chart.hidden = tableShown;
        table.hidden = !tableShown;
      }

      render(true);
      search.addEventListener("input", function () { render(false); });
      select.addEventListener("change", function () { render(false); });
    });
  });
})();
