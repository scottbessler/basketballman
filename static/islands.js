import { h, render } from "https://esm.sh/preact@10.24.3";

function attachFilters() {
  const input = document.querySelector("[data-team-filter]");
  const table = document.querySelector("[data-filter-table]");
  if (!input || !table) return;

  input.addEventListener("input", () => {
    const term = input.value.toLowerCase();
    for (const row of table.querySelectorAll("tbody tr")) {
      row.hidden = !row.textContent.toLowerCase().includes(term);
    }
  });
}

function valueFor(cell) {
  const text = (cell?.innerText || "").trim();
  if (!text || text === "-") return { kind: "empty", value: "" };
  const record = text.match(/^(\d+)-(\d+)$/);
  if (record) {
    const made = Number(record[1]);
    const attempted = Number(record[2]);
    return { kind: "number", value: attempted === 0 ? 0 : made / attempted };
  }
  const numeric = Number(text.replace(/[%,$]/g, ""));
  if (Number.isFinite(numeric) && /^[-+]?[\d,.]+%?$/.test(text)) {
    return { kind: "number", value: numeric };
  }
  return { kind: "text", value: text.toLocaleLowerCase() };
}

function applySort(table, index, direction) {
  const headers = Array.from(table.tHead?.rows?.[0]?.cells || []);
  const body = table.tBodies[0];
  const header = headers[index];
  if (!body || !header) return;

  headers.forEach((item) => {
    item.dataset.sortDir = "";
    item.setAttribute("aria-sort", "none");
  });
  header.dataset.sortDir = direction;
  header.setAttribute("aria-sort", direction === "asc" ? "ascending" : "descending");

  const rows = Array.from(body.rows);
  rows.sort((a, b) => {
    const left = valueFor(a.cells[index]);
    const right = valueFor(b.cells[index]);
    if (left.kind === "empty" && right.kind !== "empty") return 1;
    if (right.kind === "empty" && left.kind !== "empty") return -1;
    const result = left.kind === "number" && right.kind === "number"
      ? left.value - right.value
      : String(left.value).localeCompare(String(right.value), undefined, { numeric: true });
    return direction === "asc" ? result : -result;
  });
  rows.forEach((row) => body.appendChild(row));
}

function attachSort() {
  document.querySelectorAll("table.sortable").forEach((table) => {
    const headers = Array.from(table.tHead?.rows?.[0]?.cells || []);
    if (!table.tBodies[0]) return;

    headers.forEach((header, index) => {
      header.tabIndex = 0;
      header.setAttribute("role", "button");
      header.setAttribute("aria-sort", "none");

      const sort = () => {
        const nextDir = header.dataset.sortDir === "asc" ? "desc" : "asc";
        applySort(table, index, nextDir);
      };

      header.addEventListener("click", sort);
      header.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          sort();
        }
      });
    });
  });

  document.querySelectorAll("table.sortable[data-default-sort-index]").forEach((table) => {
    const index = Number(table.dataset.defaultSortIndex);
    if (!Number.isInteger(index)) return;
    applySort(table, index, table.dataset.defaultSortDir === "desc" ? "desc" : "asc");
  });
}

function SimStatus() {
  return h("span", { class: "pill" }, "Simulating");
}

function attachSimFeedback() {
  for (const form of document.querySelectorAll("[data-sim-form]")) {
    form.addEventListener("submit", () => {
      const button = form.querySelector("button");
      if (button) {
        button.disabled = true;
        render(h(SimStatus), form);
      }
    });
  }
}

attachFilters();
attachSort();
attachSimFeedback();
