//! A network tap's end state, read on the machine (docs/design/
//! goatbuilder-build-targets.md section 3.8): `doctor --target <type> --site
//! <site> --live`. `ops/scripts/tap-endstate.py` runs on each machine over
//! Ansible, reads, and changes nothing; the judging is here. A fact that could
//! not be read is unknown, which is not a pass.

use std::path::Path;

use serde_json::Value;

use crate::push;
use crate::tap::{Mode, Tap};
use crate::Check;

/// The collector, under the checkout.
const SCRIPT: &str = "ops/scripts/tap-endstate.py";

/// One end-state check's result: the words, and whether it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Read one machine: the facts its own `tap-endstate.py` printed, or why not.
fn read(repo: &Path, inv: &[String], node: &str, tap: &Tap) -> Result<Value, String> {
    let mut args = inv.to_vec();
    args.extend([
        node.to_string(),
        "-m".into(),
        "script".into(),
        "-a".into(),
        format!("executable=/usr/bin/python3 {} --ports {} --management {}", repo.join(SCRIPT).display(), tap.ports.join(","), tap.management),
    ]);
    let out = push::ansible_cmd(repo, "ansible", &args).output().map_err(|e| format!("could not start ansible: {e}"))?;
    facts(&String::from_utf8_lossy(&out.stdout))
}

/// The collector's object inside Ansible's `host | CHANGED => {...}` answer.
fn facts(answer: &str) -> Result<Value, String> {
    let first = answer.lines().find(|l| !l.trim().is_empty()).unwrap_or("ansible answered nothing").trim();
    let bad = || first.chars().take(160).collect::<String>();
    if !first.contains("| CHANGED") && !first.contains("| SUCCESS") {
        return Err(bad());
    }
    let at = answer.find('{').ok_or_else(bad)?;
    let v: Value = serde_json::from_str(answer[at..].trim()).map_err(|_| bad())?;
    let text = v.get("stdout").and_then(|s| s.as_str()).unwrap_or("");
    serde_json::from_str(text.trim()).map_err(|e| format!("{SCRIPT} did not print its facts: {e}"))
}

/// The `--live` rows of one tap machine.
pub(crate) fn rows(repo: &Path, inv: &[String], node: &str, tap: &Tap) -> Vec<Check> {
    let named = |r: Row| Check {
        name: format!("tap {node} {}", r.name),
        ok: r.ok,
        detail: r.detail,
        fix: (!r.ok).then(|| "correct it on the machine, with whatever installed the tap (docs/design/goatbuilder-build-targets.md section 3.8), then run this again".to_string()),
    };
    match read(repo, inv, node, tap) {
        Ok(f) => judge(tap, &f).into_iter().map(named).collect(),
        Err(why) => vec![named(Row { name: "machine", ok: false, detail: format!("unknown: {node} was not read: {why}") })],
    }
}

fn error(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("not reported".into()),
        _ => v.get("error").and_then(|e| e.as_str()).map(String::from),
    }
}

/// Collects what is wrong and what could not be read.
#[derive(Default)]
struct Found {
    bad: Vec<String>,
    unknown: Vec<String>,
}

impl Found {
    /// The value, unless it could not be read.
    fn known<'a>(&mut self, what: &str, v: &'a Value) -> Option<&'a Value> {
        match error(v) {
            Some(e) => {
                self.unknown.push(format!("{what}: {e}"));
                None
            }
            None => Some(v),
        }
    }

    fn row(self, name: &'static str, good: String) -> Row {
        if !self.bad.is_empty() {
            return Row { name, ok: false, detail: self.bad.into_iter().chain(self.unknown.into_iter().map(|u| format!("unknown: {u}"))).collect::<Vec<_>>().join("; ") };
        }
        if !self.unknown.is_empty() {
            return Row { name, ok: false, detail: format!("unknown: {}", self.unknown.join("; ")) };
        }
        Row { name, ok: true, detail: good }
    }
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Hold one machine's facts to its declaration. Every row of section 3.8 that
/// the machine itself shows; the recording's bound and the link's far end are not.
pub(crate) fn judge(tap: &Tap, f: &Value) -> Vec<Row> {
    let ports = &tap.ports;
    let all = f["interfaces"].as_object().cloned().unwrap_or_default();
    let bridged: Vec<&String> = all.keys().filter(|k| !ports.contains(k)).collect();
    let names = ports.join(", ");
    let mut out = Vec::new();

    let mut a = Found::default();
    for p in ports.iter().chain(bridged.iter().copied()) {
        let i = all.get(p).unwrap_or(&Value::Null);
        if i["exists"].as_bool() != Some(true) {
            a.bad.push(format!("the machine has no interface {p}"));
            continue;
        }
        if let Some(v) = a.known(&format!("{p}'s addresses"), &i["addresses"]) {
            let got = strs(v);
            if !got.is_empty() {
                a.bad.push(format!("{p} has {}", got.join(", ")));
            }
        }
        if let Some(v) = a.known(&format!("{p}'s IPv6 setting"), &i["ipv6_disabled"]) {
            if v.as_i64() != Some(1) {
                a.bad.push(format!("IPv6 is not disabled on {p}"));
            }
        }
    }
    out.push(a.row("addresses", format!("no address on {names}{}, IPv6 disabled", bridged.iter().map(|b| format!(" or {b}")).collect::<String>())));

    let mut s = Found::default();
    if let Some(v) = s.known("programs that send", &f["senders"]) {
        s.bad.extend(strs(v).into_iter().map(|x| format!("{x} sends on a tap port")));
    }
    out.push(s.row("silent", "no DHCP client, router advertisement or LLDP program or setting found for the tap's interfaces".into()));

    let mut m = Found::default();
    if f["management"].as_bool() != Some(true) {
        m.bad.push(format!("the machine has no interface {}", tap.management));
    }
    if let Some(v) = m.known("the routes", &f["routes"]) {
        let routes = v.as_array().cloned().unwrap_or_default();
        let dev = |r: &Value| r["dev"].as_str().unwrap_or("").to_string();
        let mut defaults: Vec<String> = routes.iter().filter(|r| r["dst"] == "default").map(dev).collect();
        defaults.sort();
        defaults.dedup();
        if defaults.is_empty() {
            m.bad.push("there is no default route".into());
        }
        // The mesh's own interface may carry a default of its own; a tap port never does.
        if !defaults.is_empty() && !defaults.contains(&tap.management) {
            m.bad.push(format!("the default route is on {}, not on {}", defaults.join(", "), tap.management));
        }
        let mut on: Vec<String> = routes.iter().map(dev).filter(|d| ports.contains(d) || bridged.contains(&d)).collect();
        on.sort();
        on.dedup();
        for d in on {
            m.bad.push(format!("a route uses {d}, which carries the link"));
        }
    }
    out.push(m.row("management", format!("the default route is on {}, and no route uses {names}", tap.management)));

    match tap.mode {
        Mode::Listen => {
            let mut l = Found::default();
            for p in ports {
                let i = all.get(p).unwrap_or(&Value::Null);
                if let Some(b) = i["master"].as_str() {
                    l.bad.push(format!("{p} is in {b}: a listening tap holds no port in a bridge"));
                }
                let e = &f["egress"][p];
                if e["nft"] != true && e["tc"] != true {
                    let unread: Vec<String> = [("nftables", &e["nft"]), ("traffic control", &e["tc"])].iter().filter_map(|(n, v)| error(v).map(|w| format!("{n}: {w}"))).collect();
                    if unread.is_empty() {
                        l.bad.push(format!("{p}'s egress is not dropped (no nftables netdev egress drop, and no traffic-control egress drop)"));
                    } else {
                        l.unknown.push(format!("whether {p}'s egress is dropped ({})", unread.join("; ")));
                    }
                }
            }
            out.push(l.row("listen", format!("{names} in no bridge, and every frame they would send is dropped")));
        }
        Mode::Inline => {
            let mut b = Found::default();
            let mut want = ports.clone();
            want.sort();
            let holders: Vec<(&String, &Value)> = f["bridges"].as_object().map(|m| m.iter().filter(|(_, v)| strs(&v["ports"]).iter().any(|p| ports.contains(p))).collect()).unwrap_or_default();
            match &holders[..] {
                [(name, v)] => {
                    let mut got = strs(&v["ports"]);
                    got.sort();
                    if got != want {
                        b.bad.push(format!("{name} holds {}, not exactly {names}", got.join(", ")));
                    }
                    for (key, is, words) in [("stp", 0, "STP is on"), ("forward_delay", 0, "its forward delay is not zero")] {
                        if let Some(x) = b.known(&format!("{name}'s {key}"), &v[key]) {
                            if x.as_i64() != Some(is) {
                                b.bad.push(format!("{name}: {words}"));
                            }
                        }
                    }
                    if let Some(x) = b.known(&format!("{name}'s group_fwd_mask"), &v["group_fwd_mask"]) {
                        // Bit 14: the link-local group address LLDP uses (IEEE 802.1AB).
                        if x.as_i64().unwrap_or(0) & 0x4000 == 0 {
                            b.bad.push(format!("{name} does not forward LLDP (group_fwd_mask lacks 0x4000)"));
                        }
                    }
                }
                [] => b.bad.push(format!("no bridge holds {names}, so the link is down")),
                many => b.bad.push(format!("{} bridges hold the ports: {}", many.len(), many.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", "))),
            }
            out.push(b.row("inline", format!("one bridge of exactly {names}, STP off, forward delay zero, LLDP forwarded")));
        }
    }

    let mut o = Found::default();
    let mut speeds = Vec::new();
    for p in ports {
        let i = all.get(p).unwrap_or(&Value::Null);
        if let Some(v) = o.known(&format!("{p}'s offloads"), &i["offloads"]) {
            let on: Vec<&str> = v.as_object().map(|m| m.iter().filter(|(_, x)| *x == "on").map(|(k, _)| k.as_str()).collect()).unwrap_or_default();
            if !on.is_empty() {
                o.bad.push(format!("{p} has {} on", on.join(", ")));
            }
            if v["generic-receive-offload"].is_null() {
                o.unknown.push(format!("{p}'s generic-receive-offload: not reported"));
            }
        }
        if let Some(v) = o.known(&format!("{p}'s Energy-Efficient Ethernet"), &i["eee"]) {
            if !matches!(v.as_str(), Some("disabled" | "not supported")) {
                o.bad.push(format!("{p} has Energy-Efficient Ethernet {}", v.as_str().unwrap_or("on")));
            }
        }
        match i["speed"].as_i64().filter(|s| *s > 0) {
            Some(s) => {
                speeds.push(format!("{p} at {s} Mb/s"));
                if tap.source.as_deref() == Some("copper-tap") && s > 100 {
                    o.bad.push(format!("{p} is at {s} Mb/s, and a passive copper tap runs at 100 Mb/s"));
                }
            }
            None => o.unknown.push(format!("{p}'s speed: no link")),
        }
    }
    out.push(o.row("offloads", format!("receive offloads and Energy-Efficient Ethernet off; {}", speeds.join(", "))));

    let mut r = Found::default();
    if let Some(v) = r.known("the packet sockets", &f["packet_sockets"]) {
        let on = strs(v);
        for p in ports.iter().filter(|p| !on.contains(p)) {
            if tap.mode == Mode::Listen {
                r.bad.push(format!("nothing records {p}"));
            }
        }
        if tap.mode == Mode::Inline && !ports.iter().any(|p| on.contains(p)) {
            r.bad.push(format!("nothing records {}", ports.join(" or ")));
        }
        for b in bridged.iter().filter(|b| on.contains(b)) {
            r.bad.push(format!("a recorder is on {b}, which does not see frames the bridge passes from port to port"));
        }
    }
    out.push(r.row("recording", "a recorder is on the port, not on a bridge".into()));

    let mut w = Found::default();
    for p in ports {
        if let Some(v) = w.known(&format!("whether boot waits for {p}"), &f["boot_waits"][p]) {
            if v.as_bool() != Some(false) {
                w.bad.push(format!("boot waits for {p} (its systemd-networkd file does not say RequiredForOnline=no)"));
            }
        }
    }
    out.push(w.row("boot", format!("boot waits for none of {names}")));

    let mut d = Found::default();
    if let Some(v) = d.known("the watchdog", &f["watchdog"]["timeout"]) {
        if matches!(v.as_str(), None | Some("" | "0" | "infinity" | "off")) {
            d.bad.push("systemd arms no hardware watchdog (RuntimeWatchdogSec)".into());
        }
        if f["watchdog"]["device"] != true {
            d.bad.push("the machine has no watchdog device".into());
        }
    }
    out.push(d.row("watchdog", format!("a hardware watchdog restarts a hung machine ({})", f["watchdog"]["timeout"].as_str().unwrap_or(""))));

    let mut e = Found::default();
    if let Some(v) = e.known("the root's encryption", &f["root_encrypted"]) {
        if v.as_bool() != Some(true) {
            e.bad.push("the root is not encrypted, and the type requires it".into());
        }
    }
    out.push(e.row("root", "the root is on an encrypted device".into()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tap(mode: Mode) -> Tap {
        Tap { mode, source: (mode == Mode::Listen).then(|| "copper-tap".to_string()), ports: vec!["tap0".into(), "tap1".into()], management: "mgmt0".into() }
    }

    fn iface(master: Option<&str>) -> Value {
        json!({"exists": true, "addresses": [], "ipv6_disabled": 1, "master": master, "speed": 100, "offloads": {"generic-receive-offload": "off", "large-receive-offload": "off", "rx-gro-hw": null}, "eee": "disabled"})
    }

    /// A tap as section 3.8 wants it.
    fn good(mode: Mode) -> Value {
        let inline = mode == Mode::Inline;
        let m = inline.then_some("br0");
        let mut i = json!({"tap0": iface(m), "tap1": iface(m)});
        if inline {
            i["br0"] = iface(None);
        }
        json!({
            "interfaces": i, "management": true,
            "bridges": if inline { json!({"br0": {"ports": ["tap0", "tap1"], "stp": 0, "forward_delay": 0, "group_fwd_mask": 0x4000}, "docker0": {"ports": ["x"], "stp": 1, "forward_delay": 1500, "group_fwd_mask": 0}}) } else { json!({}) },
            "routes": [{"family": "4", "dst": "default", "dev": "mgmt0"}, {"family": "4", "dst": "100.127.0.0/16", "dev": "wt0"}],
            "egress": {"tap0": {"nft": false, "tc": true}, "tap1": {"nft": true, "tc": {"error": "x"}}},
            "packet_sockets": ["tap0", "tap1"], "senders": [], "boot_waits": {"tap0": false, "tap1": false},
            "watchdog": {"timeout": "15s", "device": true}, "root_encrypted": true,
        })
    }

    fn failing(mode: Mode, f: &Value) -> Vec<(&'static str, String)> {
        judge(&tap(mode), f).into_iter().filter(|r| !r.ok).map(|r| (r.name, r.detail)).collect()
    }

    #[test]
    fn a_tap_as_designed_passes_every_row_in_both_modes() {
        for mode in [Mode::Listen, Mode::Inline] {
            assert_eq!(failing(mode, &good(mode)), vec![], "{mode:?}");
            assert_eq!(judge(&tap(mode), &good(mode)).len(), 9);
        }
    }

    #[test]
    fn each_rule_broken_on_its_own_fails_its_own_row() {
        use Mode::{Inline, Listen};
        type Case = (Mode, &'static str, Value, &'static str, &'static str);
        let cases: Vec<Case> = vec![
            (Listen, "/interfaces/tap0/addresses", json!(["192.0.2.1/24"]), "addresses", "tap0 has 192.0.2.1/24"),
            (Inline, "/interfaces/br0/addresses", json!(["fe80::1/64"]), "addresses", "br0 has fe80::1/64"),
            (Inline, "/interfaces/br0/ipv6_disabled", json!(0), "addresses", "IPv6 is not disabled on br0"),
            (Listen, "/interfaces/tap1/exists", json!(false), "addresses", "no interface tap1"),
            (Listen, "/senders", json!(["lldpd"]), "silent", "lldpd sends"),
            (Listen, "/routes/0/dev", json!("tap0"), "management", "the default route is on tap0"),
            (Listen, "/routes/1/dev", json!("tap1"), "management", "a route uses tap1"),
            (Listen, "/routes/0/dst", json!("10.0.0.0/8"), "management", "no default route"),
            (Listen, "/management", json!(false), "management", "no interface mgmt0"),
            (Listen, "/interfaces/tap0/master", json!("br9"), "listen", "tap0 is in br9"),
            (Listen, "/egress/tap0/tc", json!(false), "listen", "tap0's egress is not dropped"),
            (Inline, "/bridges/br0/ports", json!(["tap0", "tap1", "mgmt0"]), "inline", "not exactly tap0, tap1"),
            (Inline, "/bridges/br0/stp", json!(1), "inline", "STP is on"),
            (Inline, "/bridges/br0/forward_delay", json!(1500), "inline", "forward delay"),
            (Inline, "/bridges/br0/group_fwd_mask", json!(8), "inline", "does not forward LLDP"),
            (Inline, "/bridges/br0/ports", json!([]), "inline", "the link is down"),
            (Inline, "/bridges/docker0/ports", json!(["tap1"]), "inline", "2 bridges hold the ports"),
            (Listen, "/interfaces/tap0/offloads/generic-receive-offload", json!("on"), "offloads", "generic-receive-offload on"),
            (Listen, "/interfaces/tap0/eee", json!("enabled - active"), "offloads", "Energy-Efficient Ethernet enabled"),
            (Listen, "/interfaces/tap0/speed", json!(1000), "offloads", "a passive copper tap runs at 100 Mb/s"),
            (Listen, "/packet_sockets", json!(["tap0"]), "recording", "nothing records tap1"),
            (Inline, "/packet_sockets", json!(["br0"]), "recording", "nothing records tap0 or tap1"),
            (Inline, "/packet_sockets", json!(["tap0", "br0"]), "recording", "a recorder is on br0"),
            (Listen, "/boot_waits/tap0", json!(true), "boot", "boot waits for tap0"),
            (Listen, "/watchdog/timeout", json!("0"), "watchdog", "arms no hardware watchdog"),
            (Listen, "/watchdog/device", json!(false), "watchdog", "no watchdog device"),
            (Listen, "/root_encrypted", json!(false), "root", "the root is not encrypted"),
        ];
        for (mode, at, value, row, says) in cases {
            let mut f = good(mode);
            *f.pointer_mut(at).unwrap_or_else(|| panic!("{at}")) = value;
            let got = failing(mode, &f);
            assert!(got.len() == 1 && got[0].0 == row && got[0].1.contains(says), "{mode:?} {at}: {got:?}");
        }
    }

    #[test]
    fn a_fact_that_was_not_read_is_unknown_and_never_a_pass() {
        let e = json!({"error": "not permitted"});
        for (mode, at, row) in [
            (Mode::Listen, "/interfaces/tap0/addresses", "addresses"),
            (Mode::Listen, "/senders", "silent"),
            (Mode::Listen, "/routes", "management"),
            (Mode::Listen, "/egress/tap0/tc", "listen"),
            (Mode::Inline, "/bridges/br0/stp", "inline"),
            (Mode::Listen, "/interfaces/tap0/offloads", "offloads"),
            (Mode::Listen, "/packet_sockets", "recording"),
            (Mode::Listen, "/boot_waits/tap1", "boot"),
            (Mode::Listen, "/watchdog/timeout", "watchdog"),
            (Mode::Listen, "/root_encrypted", "root"),
        ] {
            for v in [e.clone(), Value::Null] {
                let mut f = good(mode);
                *f.pointer_mut(at).unwrap() = v;
                let got = failing(mode, &f);
                assert!(got.len() == 1 && got[0].0 == row && got[0].1.contains("unknown"), "{at}: {got:?}");
            }
        }
        // Nothing at all: no row passes but the ones that read a list.
        assert!(judge(&tap(Mode::Inline), &json!({})).iter().all(|r| !r.ok), "{:?}", judge(&tap(Mode::Inline), &json!({})));
    }

    #[test]
    fn ansibles_answer_is_unwrapped_and_a_refusal_is_not_facts() {
        let ok = "tap-1 | CHANGED => {\n    \"rc\": 0,\n    \"stdout\": \"{\\\"management\\\": true}\\n\"\n}\n";
        assert_eq!(facts(ok).unwrap(), json!({"management": true}));
        for bad in ["tap-1 | UNREACHABLE! => {\n \"msg\": \"no route\"\n}", "tap-1 | FAILED! => {\n \"stdout\": \"{}\"\n}", "", "tap-1 | CHANGED => {\n \"stdout\": \"Traceback\"\n}"] {
            assert!(facts(bad).is_err(), "{bad}");
        }
    }
}
