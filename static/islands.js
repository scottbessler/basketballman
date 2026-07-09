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
attachSimFeedback();
