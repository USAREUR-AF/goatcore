// ops/ory-consent/tests/double-submit-guard.test.js
//
// Behavioural test of the F-341 double-submit guard. Runs on bare node — no
// browser, no jsdom, no network — so it is cheap enough to run on every PR.
//
// verify-patched.sh proves the guard is PRESENT in the built image by grepping
// for its markers. Presence is not correctness: the properties that actually
// matter cannot be grepped for.
//
// Run:  node ops/ory-consent/tests/double-submit-guard.test.js
//
// The guard under test is READ OUT OF apply-view-patch.sh, not copied here. It
// used to be a hand-kept copy, run against a `document` stub that remembered
// ONE listener — so the test could not see the defect it now pins:
//
//   The scripts partial is not included once per page. consent.hbs includes it
//   AND renders inside layouts/auth.hbs, which includes it again. With two
//   copies of the listener, both saw the same submit event: the first marked
//   the form, the second found the mark and cancelled the FIRST submission.
//   "Allow" on the consent page sent nothing: the consent page rendered 200
//   twice for one challenge and no POST /consent left the browser. It stays
//   unseen where every other client is first-party with skip_consent, since
//   the Allow page is never shown there.
//
// Each scenario checks the same four properties:
//   1. the FIRST submit goes through
//   2. the SECOND submit of the same form is prevented
//   3. the submit control is NOT disabled synchronously (which would drop
//      method=password from the form body and break every login)
//   4. the control IS disabled after the macrotask
//
// And the cases where nothing was sent, which must never leave the page stuck
// (a faded button and no POST leaving the browser):
//   5. a script-dispatched submit event (isTrusted false: a password manager's
//      auto-submit fires one before pressing the button) is neither blocked nor
//      counted, so the real submit after it goes through
//   6. a submit another handler cancelled releases the form
//   7. a form still on screen after the release delay is released
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const src = fs.readFileSync(path.join(__dirname, "..", "apply-view-patch.sh"), "utf8");
const m = src.match(/<script>([\s\S]*?)<\/script>/);
if (!m) {
  console.log("  FAIL  could not find the guard <script> in apply-view-patch.sh");
  process.exit(1);
}
const guard = m[1];

// One page: a document that keeps EVERY capture listener, like a real one, and
// a dispatch that runs them in order and honours stopImmediatePropagation.
function page(includes) {
  const listeners = [];
  const timers = [];
  const button = {
    _a: {}, style: {},
    setAttribute(n, v) { this._a[n] = v; },
    removeAttribute(n) { delete this._a[n]; },
  };
  const form = {
    tagName: "FORM",
    _attrs: {},
    hasAttribute(n) { return n in this._attrs; },
    setAttribute(n, v) { this._attrs[n] = v; },
    removeAttribute(n) { delete this._attrs[n]; },
    querySelectorAll() { return [button]; },
  };
  const window = { setTimeout: (fn, ms) => timers.push({ fn, ms: ms || 0 }) };
  const document = { addEventListener: (t, fn) => { if (t === "submit") listeners.push(fn); } };
  const ctx = vm.createContext({ window, document });
  for (let i = 0; i < includes; i++) vm.runInContext(guard, ctx);

  // trusted: fired by the browser. cancelBy: another handler that cancels it.
  const submit = ({ trusted = true, cancelledByPage = false } = {}) => {
    const ev = {
      target: form, isTrusted: trusted, defaultPrevented: false, stopped: false,
      get prevented() { return this.defaultPrevented; },
      preventDefault() { this.defaultPrevented = true; },
      stopImmediatePropagation() { this.stopped = true; },
    };
    for (const fn of listeners) { if (ev.stopped) break; fn(ev); }
    if (cancelledByPage && !ev.stopped) ev.preventDefault();
    return ev;
  };
  // Run the macrotasks: `immediate` only the 0 ms ones, otherwise all of them.
  const flush = (immediate = true) => {
    const due = timers.filter((t) => !immediate || t.ms === 0);
    for (const t of due) timers.splice(timers.indexOf(t), 1);
    due.sort((a, b) => a.ms - b.ms).forEach((t) => t.fn());
  };
  const disabled = () => button._a["aria-disabled"] === "true";
  return { submit, button, form, flush, disabled, listeners };
}

let ok = true;
const check = (name, pass) => { console.log(`  ${pass ? "PASS" : "FAIL"}  ${name}`); if (!pass) ok = false; };

for (const [label, includes] of [
  ["partial included once (login page)", 1],
  ["partial included twice (consent page: view + layout)", 2],
]) {
  console.log(label);
  const p = page(includes);
  const first = p.submit();
  const disabledSync = p.disabled();
  const second = p.submit();
  p.flush();
  check("one listener registered, however many includes", p.listeners.length === 1);
  check("first submit allowed through", first.prevented === false);
  check("second submit PREVENTED", second.prevented === true);
  check("control NOT disabled synchronously", disabledSync === false);
  check("control disabled after the macrotask", p.disabled() === true);
}

console.log("script-dispatched submit event, then the real submit (password manager auto-submit)");
{
  const p = page(1);
  const synthetic = p.submit({ trusted: false });
  p.flush();
  check("script event not blocked", synthetic.prevented === false);
  check("script event does not mark the form", !p.form.hasAttribute("data-dlf-submitted") && !p.disabled());
  const real = p.submit();
  check("real submit after it goes through", real.prevented === false);
  check("a second real submit is still PREVENTED", p.submit().prevented === true);
}

console.log("first submit cancelled by another handler, then pressed again");
{
  const p = page(1);
  p.submit({ cancelledByPage: true });
  p.flush();
  check("form released after a cancelled submit", !p.form.hasAttribute("data-dlf-submitted"));
  check("control left enabled", !p.disabled());
  check("next submit goes through", p.submit().prevented === false);
}

console.log("submit sent but the page never left (lost on the network)");
{
  const p = page(1);
  p.submit();
  p.flush();
  check("control disabled while the response is awaited", p.disabled());
  p.flush(false);
  check("form released after the release delay", !p.form.hasAttribute("data-dlf-submitted"));
  check("control re-enabled", !p.disabled() && p.button.style.pointerEvents === "" && p.button.style.opacity === "");
  check("next submit goes through", p.submit().prevented === false);
}

console.log(ok ? "guard behaviour: OK" : "guard behaviour: BROKEN");
process.exit(ok ? 0 : 1);
