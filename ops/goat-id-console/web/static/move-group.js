// SPDX-License-Identifier: Apache-2.0
document.querySelectorAll("[data-move-peer]").forEach((button) => {
  const dialog = button.nextElementSibling;
  const form = dialog.querySelector("form");
  const search = form.querySelector("[data-move-group-search]");
  const destination = form.elements.namedItem("to");
  const source = form.elements.namedItem("from");
  const preview = form.querySelector("[data-move-preview]");

  const confirm = form.querySelector("[data-move-confirm]");
  const result = form.querySelector("[data-move-preview-result]");
  const resultHeading = result.querySelector("[data-move-preview-heading]");
  const resultDetail = result.querySelector("[data-move-preview-detail]");
  const declaration = form.querySelector("[data-move-declaration]");
  const fields = form.querySelector("[data-move-fields]");
  const edit = form.querySelector("[data-move-edit]");
  const dismiss = form.querySelector("[data-move-dismiss]");
  const previewURL = form.dataset.movePreviewUrl;
  const confirmURL = form.dataset.moveConfirmUrl;
  let pending;
  let confirmation = "";
  let saved = button.dataset.moveSaved ? JSON.parse(button.dataset.moveSaved) : null;
  if (saved) {
    saved.operation_id = button.dataset.moveSavedId;
    saved.expires_at = button.dataset.moveSavedExpiry;
  }

  function updatePreviewStyle() {
    const showingDeclaration = !declaration.hidden;
    fields.hidden = showingDeclaration;
    preview.hidden = showingDeclaration;
    edit.hidden = !showingDeclaration;
    dismiss.textContent = showingDeclaration ? "Close" : "Cancel";
    dismiss.classList.toggle("move-group-cancel", !showingDeclaration);
    const filled = Boolean(source.value && destination?.value);
    preview.classList.toggle("primary", filled);
    preview.disabled = showingDeclaration || !previewURL || !filled || Boolean(pending);
    confirm.hidden = !confirmURL || !confirmation;
    confirm.disabled = Boolean(pending);
  }

  function showPreviewResult(state, heading, detail = "") {
    result.dataset.state = state;
    resultHeading.textContent = heading;
    resultDetail.textContent = detail;
    result.hidden = false;
  }

  function clearDeclaration() {
    declaration.hidden = true;
    declaration.querySelector("[data-move-declaration-yaml]").value = "";
    declaration.querySelector("[data-copy-status]").textContent = "";
  }

  function showDeclaration(proposal, review) {
    const instruction = proposal.creates_fragment ? "Create this file in the configuration repository:"
      : proposal.replaces_existing ? "Update the device’s existing entry in:"
        : "Add this entry under entries: in:";
    declaration.querySelector("[data-move-declaration-instruction]").textContent = instruction;
    declaration.querySelector("[data-move-declaration-path]").textContent = proposal.fragment;
    declaration.querySelector("[data-move-declaration-yaml]").value = proposal.yaml;
    const reference = declaration.querySelector("[data-move-declaration-reference]");
    reference.hidden = !review?.operation_id;
    reference.textContent = review?.operation_id
      ? `Reference ${review.operation_id} · Available until ${review.expires_at}`
      : "";
    declaration.hidden = false;
  }

  function clearPreview() {
    pending?.abort();
    pending = undefined;
    confirmation = "";
    clearDeclaration();
    result.hidden = true;
    resultHeading.textContent = "";
    resultDetail.textContent = "";
    updatePreviewStyle();
  }

  edit.addEventListener("click", () => {
    clearPreview();
    source.focus();
  });
  source.addEventListener("change", clearPreview);
  dialog.addEventListener("close", clearPreview);

  button.addEventListener("click", () => {
    form.reset();
    if (destination) destination.value = "";
    clearPreview();
    if (saved?.expires_at && Date.parse(saved.expires_at) <= Date.now()) saved = null;
    if (saved) {
      source.value = saved.from_group;
      if (destination) {
        destination.value = saved.to_group;
        search.value = saved.to_group;
      }
      if (saved.declaration) {
        showDeclaration(saved.declaration, saved);
        showPreviewResult("success", "Declaration ready for review", "No membership changes were applied.");
      } else {
        showPreviewResult("failure", "Confirmation failed", saved.error + " Preview again after resolving the issue." +
          (saved.operation_id ? ` Saved result: ${saved.operation_id}.` : ""));
      }
      updatePreviewStyle();
    }
    dialog.showModal();
    if (saved?.declaration) declaration.querySelector("[data-copy-button]").focus();
  });

  function describePlan(plan) {
    const mover = plan.movers[0];
    const changes = mover
      ? `Move ${mover.name} from ${plan.from_group} to ${plan.to_group}.`
      : `This device is already in ${plan.to_group}.`;
    const keep = mover?.other_groups?.length ? ` Keep ${mover.other_groups.join(", ")}.` : "";
    const authority = plan.authority === "enforce"
      ? " This scope requires a reviewed warrant declaration."
      : plan.authority === "unknown" ? " Membership authority could not be verified." : "";
    return changes + keep + authority;
  }

  async function reviewMove(isConfirmation) {
    const body = new URLSearchParams(new FormData(form));
    if (isConfirmation) body.set("confirmation", confirmation);
    const request = new AbortController();
    pending = request;
    if (!isConfirmation) confirmation = "";
    clearDeclaration();
    updatePreviewStyle();
    showPreviewResult("pending", isConfirmation ? "Rechecking plan…" : "Checking move…");
    let operationID;
    let expiresAt;
    try {
      const response = await fetch(isConfirmation ? confirmURL : previewURL, {
        method: "POST", body, signal: request.signal,
      });
      operationID = response.headers.get("X-Move-Operation-ID");
      expiresAt = response.headers.get("X-Move-Expires-At");
      const isJSON = response.headers.get("Content-Type")?.startsWith("application/json");
      if (!response.ok && !(isConfirmation && response.status === 409 && isJSON)) {
        const textError = response.headers.get("Content-Type")?.startsWith("text/plain");
        throw new Error(textError ? (await response.text()).trim() : "Move review was refused. Refresh the page and retry.");
      }
      const plan = await response.json();
      if (pending !== request) return;
      confirmation = plan.confirmation_token || "";
      if (plan.confirmation_state === "changed") {
        const next = confirmation ? " Review the updated plan and confirm again." : "";
        showPreviewResult("failure", "Plan changed", describePlan(plan) + next);
      } else if (plan.confirmation_state === "declaration_ready") {
        if (plan.operation_id) saved = plan;
        showDeclaration(plan.declaration, plan);
        showPreviewResult("success", "Declaration ready for review", "No membership changes were applied.");
        declaration.querySelector("[data-copy-button]").focus();
      } else if (plan.confirmation_state === "confirmed") {
        showPreviewResult("success", "Plan confirmed", "No membership changes were applied.");
      } else {
        showPreviewResult("success", "Preview succeeded", describePlan(plan));
      }
    } catch (error) {
      if (pending === request) {
        confirmation = "";
        const detail = error instanceof TypeError ? "Move preview could not be reached. Try again." : error.message;
        if (operationID && isConfirmation) {
          saved = { from_group: body.get("from"), to_group: body.get("to"), error: detail, operation_id: operationID, expires_at: expiresAt };
        }
        showPreviewResult("failure", isConfirmation ? "Confirmation failed" : "Preview failed", detail);
      }
    } finally {
      if (pending === request) {
        pending = undefined;
        updatePreviewStyle();
      }
    }
  }

  preview.addEventListener("click", () => reviewMove(false));
  confirm.addEventListener("click", () => reviewMove(true));

  if (!search) return;

  const list = dialog.querySelector('[role="listbox"]');
  const options = Array.from(list.children);
  const noResults = form.querySelector("[data-move-group-no-results]");
  let active = null;

  function setActive(option) {
    active?.setAttribute("aria-selected", "false");
    active = option;
    if (option) {
      option.setAttribute("aria-selected", "true");
      search.setAttribute("aria-activedescendant", option.id);
      option.scrollIntoView({ block: "nearest" });
    } else {
      search.removeAttribute("aria-activedescendant");
    }
  }

  function closeList() {
    list.hidden = true;
    noResults.hidden = true;
    search.setAttribute("aria-expanded", "false");
    setActive(null);
  }

  function filterGroups() {
    const query = search.value.trim().toLowerCase();
    setActive(null);
    for (const option of options) {
      option.hidden = !option.textContent.toLowerCase().includes(query);
    }
    list.hidden = options.every((option) => option.hidden);
    search.setAttribute("aria-expanded", String(!list.hidden));
    noResults.hidden = !list.hidden;
  }

  function choose(option) {
    search.value = option.textContent;
    destination.value = search.value;
    clearPreview();
    closeList();
  }

  search.addEventListener("focus", filterGroups);
  search.addEventListener("click", filterGroups);
  search.addEventListener("input", () => {
    destination.value = "";
    clearPreview();
    filterGroups();
  });
  search.addEventListener("blur", () => {
    search.value = destination.value;
    closeList();
  });
  list.addEventListener("pointerdown", (event) => {
    if (event.target.matches('[role="option"]')) event.preventDefault();
  });
  list.addEventListener("click", (event) => {
    if (options.includes(event.target)) choose(event.target);
  });
  search.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && (!list.hidden || !noResults.hidden)) {
      event.preventDefault();
      search.value = destination.value;
      closeList();
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (active) choose(active);
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (list.hidden) filterGroups();
      const matches = options.filter((option) => !option.hidden);
      const index = matches.indexOf(active);
      const next = event.key === "ArrowDown"
        ? Math.min(index + 1, matches.length - 1)
        : index < 0 ? matches.length - 1 : Math.max(index - 1, 0);
      setActive(matches[next] ?? null);
    }
  });
  dialog.addEventListener("close", closeList);
});

const moveTooltips = document.querySelectorAll('.move-group-disabled [role="tooltip"]');

function positionMoveTooltip(tooltip) {
  const trigger = tooltip.previousElementSibling.getBoundingClientRect();
  const { width, height } = tooltip.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth;
  const viewportHeight = document.documentElement.clientHeight;
  const inset = 8;
  const left = Math.max(inset, Math.min(trigger.left, viewportWidth - width - inset));
  const preferredTop = trigger.bottom + height <= viewportHeight - inset
    ? trigger.bottom
    : trigger.top - height;
  const top = Math.max(inset, Math.min(preferredTop, viewportHeight - height - inset));
  tooltip.style.left = `${left}px`;
  tooltip.style.top = `${top}px`;
}

moveTooltips.forEach((tooltip) => {
  const wrapper = tooltip.parentElement;
  const button = tooltip.previousElementSibling;

  function show() {
    tooltip.hidden = false;
    positionMoveTooltip(tooltip);
  }

  wrapper.addEventListener("mouseenter", show);
  wrapper.addEventListener("mouseleave", () => {
    if (document.activeElement !== button) tooltip.hidden = true;
  });
  button.addEventListener("focus", show);
  button.addEventListener("blur", () => {
    if (!wrapper.matches(":hover")) tooltip.hidden = true;
  });
});

if (moveTooltips.length > 0) {
  function repositionMoveTooltips() {
    moveTooltips.forEach((tooltip) => {
      if (!tooltip.hidden) positionMoveTooltip(tooltip);
    });
  }

  window.addEventListener("scroll", repositionMoveTooltips, { capture: true, passive: true });
  window.addEventListener("resize", repositionMoveTooltips);
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    moveTooltips.forEach((tooltip) => {
      if (!tooltip.hidden) {
        event.preventDefault();
        tooltip.hidden = true;
      }
    });
  });
}
