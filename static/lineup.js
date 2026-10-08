// Lineup editor enhancements. Everything works without JS (plain form +
// up/down submit buttons); this adds drag reorder, paint-drag chart,
// live validation and slider read-outs. Vanilla JS, no external deps.

const root = document.querySelector(".lineup-page");
const form = document.querySelector("[data-lineup-form]");

if (root && form) {
  document.documentElement.classList.add("js");
  init();
}

function init() {
  const modeRadios = Array.from(form.querySelectorAll('input[name="mode"]'));
  const errorBox = document.querySelector("[data-lineup-error]");

  const currentMode = () => modeRadios.find((radio) => radio.checked)?.value || "auto";
  const syncMode = () => {
    root.classList.remove("mode-auto", "mode-minutes", "mode-chart");
    root.classList.add(`mode-${currentMode()}`);
  };
  const setMode = (mode) => {
    const radio = modeRadios.find((item) => item.value === mode);
    if (radio && !radio.checked) {
      radio.checked = true;
      syncMode();
    }
  };
  modeRadios.forEach((radio) => radio.addEventListener("change", syncMode));
  syncMode();

  // Slider read-outs.
  form.querySelectorAll('input[type="range"]').forEach((input) => {
    const output = input.closest(".slider")?.querySelector("output");
    if (!output) return;
    const update = () => {
      output.textContent = input.value;
    };
    input.addEventListener("input", update);
    update();
  });

  initDepth(setMode);
  const chart = initChart(setMode);

  form.addEventListener("submit", (event) => {
    errorBox.hidden = true;
    const submitter = event.submitter;
    const skipsValidation = submitter && (submitter.hasAttribute("formnovalidate") || submitter.name === "reset");
    if (!skipsValidation && currentMode() === "chart" && chart && !chart.valid()) {
      event.preventDefault();
      errorBox.textContent = "Every 4-minute block needs exactly five players on the floor (see the red counts).";
      errorBox.hidden = false;
      errorBox.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  });
}

function initDepth(setMode) {
  const list = document.querySelector("[data-depth]");
  if (!list) return;
  const total = document.querySelector("[data-minutes-total]");

  const rows = () => Array.from(list.querySelectorAll(".depth-row"));
  const refresh = () => {
    rows().forEach((row, index) => row.classList.toggle("starter", index < 5));
    if (total) {
      const sum = rows().reduce((acc, row) => acc + (Number(row.querySelector(".mins input")?.value) || 0), 0);
      total.textContent = `Targets sum to ${sum} (scaled to 240)`;
    }
  };

  list.addEventListener("input", (event) => {
    if (event.target.matches('.mins input')) {
      setMode("minutes");
      refresh();
    }
  });

  // Up/down buttons double as no-JS fallbacks; with JS move the row in place.
  list.addEventListener("click", (event) => {
    const button = event.target.closest("[data-move]");
    if (!button) return;
    event.preventDefault();
    const row = button.closest(".depth-row");
    if (button.dataset.move === "up" && row.previousElementSibling) {
      list.insertBefore(row, row.previousElementSibling);
    } else if (button.dataset.move === "down" && row.nextElementSibling) {
      list.insertBefore(row.nextElementSibling, row);
    }
    setMode("minutes");
    refresh();
  });

  // Pointer-based drag (mouse, pen and touch share one code path).
  let dragging = null;
  list.addEventListener("pointerdown", (event) => {
    const handle = event.target.closest(".handle");
    if (!handle) return;
    dragging = handle.closest(".depth-row");
    dragging.classList.add("dragging");
    handle.setPointerCapture(event.pointerId);
    event.preventDefault();
  });
  list.addEventListener("pointermove", (event) => {
    if (!dragging) return;
    const siblings = rows().filter((row) => row !== dragging);
    const next = siblings.find((row) => {
      const box = row.getBoundingClientRect();
      return event.clientY < box.top + box.height / 2;
    });
    if (next) {
      if (dragging.nextElementSibling !== next) list.insertBefore(dragging, next);
    } else if (list.lastElementChild !== dragging) {
      list.appendChild(dragging);
    }
    setMode("minutes");
    refresh();
  });
  const stop = () => {
    if (!dragging) return;
    dragging.classList.remove("dragging");
    dragging = null;
    refresh();
  };
  list.addEventListener("pointerup", stop);
  list.addEventListener("pointercancel", stop);
  refresh();
}

function initChart(setMode) {
  const table = document.querySelector("[data-chart]");
  if (!table) return null;
  const body = table.tBodies[0];
  const blockCount = 12;

  const boxes = () => Array.from(body.querySelectorAll("td.cell input"));
  const refresh = () => {
    const counts = new Array(blockCount).fill(0);
    for (const row of body.rows) {
      let blocks = 0;
      row.querySelectorAll("td.cell input").forEach((box, block) => {
        if (box.checked) {
          counts[block] += 1;
          blocks += 1;
        }
      });
      const cell = row.querySelector("[data-row-minutes]");
      if (cell) cell.textContent = String(blocks * 4);
    }
    counts.forEach((count, block) => {
      const cell = table.querySelector(`[data-block-count="${block}"]`);
      if (!cell) return;
      cell.textContent = String(count);
      cell.classList.toggle("bad", count !== 5);
    });
    return counts;
  };

  const setBox = (box, value) => {
    if (box.checked !== value) {
      box.checked = value;
      setMode("chart");
      refresh();
    }
  };

  // Native checkbox clicks are replaced by pointer painting; keyboard
  // activation (detail === 0) still toggles directly.
  body.addEventListener("click", (event) => {
    const box = event.target.closest("td.cell input");
    if (!box) return;
    if (event.detail === 0) {
      setMode("chart");
      queueMicrotask(refresh);
    } else {
      event.preventDefault();
    }
  });

  let paint = null;
  body.addEventListener("pointerdown", (event) => {
    const cell = event.target.closest("td.cell");
    if (!cell) return;
    const box = cell.querySelector("input");
    paint = { value: !box.checked, row: cell.parentElement };
    setBox(box, paint.value);
    try {
      body.setPointerCapture(event.pointerId);
    } catch {
      // Some browsers refuse capture on non-focusable targets; painting still works.
    }
  });
  body.addEventListener("pointermove", (event) => {
    if (!paint) return;
    const element = document.elementFromPoint(event.clientX, event.clientY);
    const cell = element?.closest?.("td.cell");
    if (cell && cell.parentElement === paint.row) {
      setBox(cell.querySelector("input"), paint.value);
    }
  });
  const stop = () => {
    paint = null;
  };
  body.addEventListener("pointerup", stop);
  body.addEventListener("pointercancel", stop);

  document.querySelector("[data-chart-clear]")?.addEventListener("click", () => {
    boxes().forEach((box) => {
      box.checked = false;
    });
    setMode("chart");
    refresh();
  });

  refresh();
  return { valid: () => refresh().every((count) => count === 5) };
}
