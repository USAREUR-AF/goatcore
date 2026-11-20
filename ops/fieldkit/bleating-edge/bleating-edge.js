/* SPDX-License-Identifier: Apache-2.0
 * Bleating Edge — the one small script every Fieldkit tool includes.
 * docs/design/fieldkit-and-bleating-edge.md §4.
 *
 * A tool declares its actions in its own HTML, one element per action:
 *   <button data-action="allow" data-cli="pod rules allow" data-api="POST /v1/rules">
 * BE.call(name, params, body) is the ONLY way a tool reaches its API, and it
 * can reach only a route an action declares. ops/ci/check-fieldkit-scope.py
 * checks every declared action against the allow list of CLI verbs, so a tool
 * cannot do anything the CLI cannot.
 *
 * No state is kept here: every list is read fresh from the API.
 */
(function () {
  "use strict";

  function h(tag, attrs) {
    var el = document.createElement(tag);
    var a = attrs || {};
    Object.keys(a).forEach(function (k) {
      var v = a[k];
      if (v === null || v === undefined || v === false) return;
      if (k === "text") el.textContent = v;
      else if (k.slice(0, 2) === "on" && typeof v === "function") el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? "" : String(v));
    });
    for (var i = 2; i < arguments.length; i++) {
      var c = arguments[i];
      if (c === null || c === undefined || c === false) continue;
      (Array.isArray(c) ? c : [c]).forEach(function (x) {
        el.append(x instanceof Node ? x : document.createTextNode(String(x)));
      });
    }
    return el;
  }

  // The action table, read from the page: name -> {cli, method, route, el}.
  function actions() {
    var out = {};
    document.querySelectorAll("[data-action]").forEach(function (el) {
      var api = (el.getAttribute("data-api") || "").trim().split(/\s+/);
      out[el.getAttribute("data-action")] = {
        cli: el.getAttribute("data-cli") || "",
        method: api[0] || "GET",
        route: api[1] || "",
        el: el,
      };
    });
    return out;
  }

  function fillRoute(route, params) {
    return route.replace(/\{(\w+)\}/g, function (_, k) {
      if (!params || params[k] === undefined || params[k] === "") throw plainError("The page is missing a value: " + k + ".");
      return encodeURIComponent(params[k]);
    });
  }

  function plainError(what, why, fix, code) {
    var e = new Error(what);
    e.what = what;
    e.why = why || "";
    e.fix = fix || "";
    e.code = code || "";
    return e;
  }

  // Call one declared action. Resolves to the parsed JSON body.
  function call(name, params, body) {
    var a = actions()[name];
    if (!a || !a.route) return Promise.reject(plainError("This page has no action named " + name + "."));
    var path;
    try { path = fillRoute(a.route, params); } catch (e) { return Promise.reject(e); }
    var opts = { method: a.method, headers: { Accept: "application/json" }, credentials: "same-origin", cache: "no-store" };
    if (body !== undefined) {
      opts.headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    return fetch(path, opts).then(function (res) {
      return res.text().then(function (text) {
        var data = null;
        try { data = text ? JSON.parse(text) : {}; } catch (_) { data = null; }
        if (res.ok && data) return data;
        var err = data && data.error ? data.error : {};
        throw plainError(
          err.what || err.message || ("The pod refused (" + res.status + ")."),
          err.why || "",
          err.fix || (res.status === 401 || res.status === 403
            ? "Sign in with your goat certificate or card. Only a person named as local authority can change rules."
            : "Try again. If it fails again, give the code below to your goat operator."),
          err.code || ("HTTP " + res.status)
        );
      });
    }, function () {
      throw plainError(
        "The pod did not answer.",
        "This page talks only to the pod that served it. The pod service may have stopped or restarted.",
        "Wait one minute and press Refresh. If it still fails, check the pod has power and its service is running.",
        "NO-ANSWER"
      );
    });
  }

  // ---- time --------------------------------------------------------------
  function untilFromHours(hours, now) {
    var t = new Date((now || Date.now()) + hours * 3600 * 1000);
    t.setUTCSeconds(0, 0);
    return t.toISOString().replace(".000Z", "Z");
  }

  function age(seconds) {
    var s = Math.max(0, Math.round(seconds));
    if (s < 90) return s + " seconds";
    if (s < 90 * 60) return Math.round(s / 60) + " minutes";
    if (s < 48 * 3600) return Math.round(s / 3600) + " hours";
    return Math.round(s / 86400) + " days";
  }

  function when(iso) {
    if (!iso) return "no end date";
    var d = new Date(iso);
    if (isNaN(d.getTime())) return iso;
    var diff = (d.getTime() - Date.now()) / 1000;
    var rel = diff >= 0 ? "in " + age(diff) : age(-diff) + " ago";
    var two = function (n) { return (n < 10 ? "0" : "") + n; };
    var abs = d.getFullYear() + "-" + two(d.getMonth() + 1) + "-" + two(d.getDate()) + " " + two(d.getHours()) + ":" + two(d.getMinutes());
    return abs + " (" + rel + ")";
  }

  // ---- components ----------------------------------------------------------

  // 1. Connected or cut-off banner.
  function banner(el, s) {
    el.replaceChildren();
    if (!s) {
      el.setAttribute("data-state", "unknown");
      el.append("Pod state not known yet.");
      return;
    }
    el.setAttribute("data-state", s.connected ? "connected" : "cutoff");
    el.append(s.connected ? "Connected to goat." : "Cut off from goat. This pod works on its own.");
    var detail = s.connected
      ? "Rules came from goat " + age(s.generation_age_seconds || 0) + " ago."
      : "Last rules from goat are " + age(s.generation_age_seconds || 0) + " old. Changes made here wait for an org lead to check them.";
    el.append(h("small", { text: detail }));
  }

  // 2. List row for a source or rule.
  function row(o) {
    var li = h("li", { class: "be-row", "data-key": o.key || null },
      h("div", { class: "be-row-main" },
        h("div", { class: "be-row-title", text: o.title }),
        o.meta ? h("div", { class: "be-row-meta", text: o.meta }) : null),
      o.badge ? h("span", { class: "be-badge", "data-tone": o.badge.tone || "", text: o.badge.text }) : null,
      o.button ? h("button", { type: "button", text: o.button.label, onclick: o.button.onClick }) : null);
    return li;
  }

  // 4. Duration picker. Hours above `max` are disabled.
  function duration(container, name, max, choices) {
    var hours = choices || [4, 24, 72];
    var fs = h("fieldset", { class: "be-duration" }, h("legend", { text: "For how long" }));
    var first = true;
    hours.forEach(function (n) {
      var off = typeof max === "number" && n > max;
      var input = h("input", { type: "radio", name: name, value: String(n), disabled: off, checked: !off && first });
      if (!off) first = false;
      fs.append(h("label", null, input, h("span", { text: n + " hours" })));
    });
    container.replaceChildren(fs);
    return {
      hours: function () {
        var c = fs.querySelector("input:checked");
        return c ? Number(c.value) : null;
      },
      onChange: function (fn) { fs.addEventListener("change", fn); },
    };
  }

  // Shell-quote one argument for the CLI strip.
  function quote(s) {
    s = String(s);
    if (/^[A-Za-z0-9._:\/=@+-]+$/.test(s)) return s;
    return "'" + s.replace(/'/g, "'\\''") + "'";
  }

  // 6. "Same as" CLI strip. `words` are the arguments after the verb.
  function cli(el, verb, words) {
    var text = "goat " + verb + (words && words.length ? " " + words.map(quote).join(" ") : "");
    el.replaceChildren(
      h("span", { class: "be-cli-label", text: "Same as" }),
      h("code", { text: text }),
      h("button", {
        type: "button", text: "Copy", "aria-label": "Copy the command",
        onclick: function (ev) {
          var b = ev.currentTarget;
          var done = function () { b.textContent = "Copied"; setTimeout(function () { b.textContent = "Copy"; }, 1500); };
          if (navigator.clipboard && window.isSecureContext) navigator.clipboard.writeText(text).then(done, function () {});
        },
      }));
    return text;
  }

  // 5. Confirm sheet. Resolves true only when the person presses the yes button.
  function confirm(o) {
    return new Promise(function (resolve) {
      var dlg = h("dialog", { class: "be-sheet", "aria-labelledby": "be-sheet-title" });
      var stripEl = h("div", { class: "be-cli" });
      var yes = h("button", { type: "button", class: "be-primary", text: o.yes || "Yes, do it" });
      var no = h("button", { type: "button", text: "No, go back" });
      dlg.append(
        h("h2", { id: "be-sheet-title", text: o.title }),
        h("div", { class: "be-preview", text: o.preview }),
        stripEl,
        h("div", { class: "be-sheet-buttons" }, no, yes));
      if (o.verb) cli(stripEl, o.verb, o.words || []);
      else stripEl.remove();
      var finish = function (v) { dlg.close(); dlg.remove(); resolve(v); };
      yes.addEventListener("click", function () { finish(true); });
      no.addEventListener("click", function () { finish(false); });
      dlg.addEventListener("cancel", function (ev) { ev.preventDefault(); finish(false); });
      document.body.append(dlg);
      if (dlg.showModal) dlg.showModal(); else dlg.setAttribute("open", "");
      no.focus();
    });
  }

  // 8. Error with what-to-do text.
  function error(el, e) {
    el.replaceChildren();
    if (!e) return;
    var dl = h("dl", null,
      h("dt", { text: "What happened" }), h("dd", { text: e.what || e.message || "Something went wrong." }));
    if (e.why) dl.append(h("dt", { text: "Why" }), h("dd", { text: e.why }));
    dl.append(h("dt", { text: "What to do" }), h("dd", { text: e.fix || "Try again." }));
    if (e.code) dl.append(h("dt", { text: "Code" }), h("dd", null, h("code", { text: e.code })));
    el.append(dl);
    el.setAttribute("role", "alert");
  }

  function note(el, text) {
    el.textContent = text || "";
    if (text) el.setAttribute("role", "status");
  }

  window.BE = {
    h: h, actions: actions, call: call, untilFromHours: untilFromHours, age: age, when: when,
    banner: banner, row: row, duration: duration, quote: quote, cli: cli, confirm: confirm,
    error: error, note: note,
  };
})();
