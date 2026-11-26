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

/// The goatnet that has `group` as a member.
pub(crate) fn goatnet_of(repo: &Path, group: &str) -> Option<String> {
    let table = crate::pins::load_table(&repo.join("ops/pins/fleet-pins.yaml")).ok()?;
    let has = |g: &serde_json::Value| g.get("members").and_then(|m| m.as_array()).is_some_and(|m| m.iter().any(|x| x.as_str() == Some(group)));
    table.goatnets.iter().find(|(_, g)| has(g)).map(|(n, _)| n.clone())
}
