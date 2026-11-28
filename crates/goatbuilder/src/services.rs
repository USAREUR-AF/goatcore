//! The services a target type lists (`services:`, docs/design/
//! goatbuilder-build-targets.md section 3.9): components of
//! ops/pins/fleet-pins.yaml its machines run. `join-node` deploys each to the
//! machine through `deploy` itself, and `doctor --target` checks them.

use std::path::Path;

use anyhow::Result;

use crate::push;

/// A listed service that its type cannot have.
const CODE: &str = "GB-384";

pub(crate) enum Service {
    /// Declared `absent: true` for the group: the pins' own word.
    Absent,
    Pinned(String),
}

/// The service `component` of `type_name`, for `group` on `goatnet`: GB-384 when
/// the pins have no component of that name, or no pin of it for the group. A list
/// is a requirement, so a missing pin is refused and never skipped.
pub(crate) fn on(repo: &Path, type_name: &str, component: &str, goatnet: &str, group: &str) -> Result<Service> {
    let bad = |why: String| -> anyhow::Error { goat_errors::coded(CODE, format!("{type_name} lists the service {component}, and {why}")).into() };
    let path = repo.join("ops/pins/fleet-pins.yaml");
    if !path.is_file() {
        return Err(bad("this checkout has no ops/pins/fleet-pins.yaml".into()));
    }
    let table = crate::pins::load_table(&path)?;
    if !table.components.contains_key(component) {
        return Err(bad("ops/pins/fleet-pins.yaml declares no component of that name".into()));
    }
    let d = push::declared(repo, component, goatnet, None).map_err(|e| bad(format!("{e:#}")))?;
    if d.skipped.iter().any(|g| g == group) {
        return Ok(Service::Absent);
    }
    if !d.members.iter().any(|g| g == group) {
        return Err(bad(format!("{group} is not a member group of the goatnet {goatnet:?}")));
    }
    let Some(pin) = push::want(repo, group, &d.var) else {
        return Err(bad(format!("ops/ansible/group_vars/{group}/pins.yml has no {} (declare its pin, then: goatbuilder pins)", d.var)));
    };
    push::resolve(repo, component, goatnet, Some(group)).map_err(|e| bad(format!("{e:#}")))?;
    Ok(Service::Pinned(pin))
}

/// What the rollout of `component` needs acknowledged, if anything (ADR 1211):
/// deploy refuses it without `--ack`, so the verb that builds a machine asks
/// for the name before it changes the machine.
pub(crate) fn needs_ack(repo: &Path, component: &str) -> Option<&'static str> {
    let d = crate::preflight::declaration(repo, component).ok()?;
    crate::preflight::needs_ack(&crate::preflight::grade_facts(&d))
}

/// Where a verb that builds a machine deploys a service: the goatnet, the
/// type's group, and the one machine when it builds one.
pub(crate) struct Onto<'a> {
    pub goatnet: &'a str,
    pub group: &'a str,
    pub node: Option<&'a str>,
}

/// The deploy of `component` onto `to`, as `verb` (the verb that builds a
/// machine) prints and runs it from the checkout `r`: with `--ack` when its
/// rollout needs one and the person gave it. The reason for GB-421 beside it
/// when it needs one and none was given, so that no phase runs.
pub(crate) fn deploy_command(repo: &Path, r: &str, verb: &str, component: &str, to: &Onto, ack: &str) -> (String, Option<String>) {
    let mut cmd = format!("\"${{GOATBUILDER_BIN:-goatbuilder}}\" --yes -C '{r}' deploy --env {} --component {component} --group {}", to.goatnet, to.group);
    if let Some(n) = to.node {
        cmd.push_str(&format!(" --node {n}"));
    }
    let Some(kind) = needs_ack(repo, component) else { return (cmd, None) };
    if ack.trim().is_empty() {
        return (cmd, Some(format!("the rollout of the service {component} is {kind}: not cleanly reversible, so its deploy needs a name that accepts the risk. Pass --ack \"<your name>\" to {verb}: it passes it to that deploy, whose ledger row records it")));
    }
    cmd.push_str(&format!(" --ack {}", crate::standup::sh_quote(ack)));
    (cmd, None)
}

/// GB-421: the refusal [`deploy_command`] gave, before any phase.
pub(crate) fn ack_refusal(why: String) -> anyhow::Error {
    goat_errors::coded("GB-421", format!("refused before any phase: {why}")).into()
}

/// The goatnet that has `group` as a member.
pub(crate) fn goatnet_of(repo: &Path, group: &str) -> Option<String> {
    let table = crate::pins::load_table(&repo.join("ops/pins/fleet-pins.yaml")).ok()?;
    let has = |g: &serde_json::Value| g.get("members").and_then(|m| m.as_array()).is_some_and(|m| m.iter().any(|x| x.as_str() == Some(group)));
    table.goatnets.iter().find(|(_, g)| has(g)).map(|(n, _)| n.clone())
}
