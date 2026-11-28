//! goat-ingress: the ingress adapter for keyless sources (ADR 1251,
//! docs/design/pod-carriage-and-provenance.md §3, §7.3).
//!
//! A keyless source (a radar, a legacy feed) is enrolled with its own slot and
//! a custodian but holds no key. The adapter is a machine with its own slot
//! and certificate. For each declared source it listens on one UDP port,
//! accepts datagrams only from the declared sender, checks ASTERIX block
//! framing, and publishes each frame under the source's slot on
//!
//! ```text
//! <source-slot>/stream/asterix/<cat>/raw/v1            (volatile = false, default)
//! volatile/<source-slot>/stream/asterix/<cat>/raw/v1   (volatile = true)
//! ```
//!
//! with an `adapter-attested` `prov` in the sample attachment naming the
//! source slot, the interface, the port and the sender address.

pub mod asterix;

use goat_provenance::{attach, sign, SignedGrade, Signer, Via};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The adapter configuration file (TOML).
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The adapter's own goat context directory, as `goat` writes it:
    /// `profile.toml` (with `router_endpoint`), `mtls.cert.pem`,
    /// `mtls.key.pem`, `ca-roots.pem`.
    pub context: PathBuf,
    /// Overrides `router_endpoint` from the context's `profile.toml`.
    #[serde(default)]
    pub router: Option<String>,
    #[serde(rename = "source", default)]
    pub sources: Vec<SourceConfig>,
}

/// One keyless source, as declared at its enrollment (§7.3).
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    /// The keyless source's slot (32 hex).
    pub slot: String,
    /// Local address and port to receive on, e.g. `0.0.0.0:30048`.
    pub listen: String,
    /// The interface name recorded in `via.iface`.
    #[serde(default = "default_interface")]
    pub interface: String,
    /// The only sender accepted: an address, or an address and port.
    pub sender: String,
    /// `asterix:<cat>,<cat>…`; frames carrying any other category are dropped.
    /// Plain `asterix` accepts every category.
    #[serde(default = "default_format")]
    pub format: String,
    /// Publish under `volatile/<slot>/…` (routed, never stored, ADR 1032).
    #[serde(default)]
    pub volatile: bool,
}

fn default_interface() -> String {
    "udp".to_string()
}
fn default_format() -> String {
    "asterix".to_string()
}

/// The declared sender: an address, or an address and one port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    Addr(IpAddr),
    AddrPort(SocketAddr),
}

impl Sender {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let s = s.trim();
        if let Ok(sa) = s.parse::<SocketAddr>() {
            return Ok(Sender::AddrPort(sa));
        }
        if let Ok(ip) = s.parse::<IpAddr>() {
            return Ok(Sender::Addr(ip));
        }
        anyhow::bail!("sender {s:?} is neither an address nor address:port")
    }

    /// True when a datagram from `from` is from the declared sender. An
    /// IPv4-mapped IPv6 address compares as its IPv4 form.
    pub fn matches(&self, from: SocketAddr) -> bool {
        let ip = from.ip().to_canonical();
        match self {
            Sender::Addr(a) => a.to_canonical() == ip,
            Sender::AddrPort(sa) => sa.ip().to_canonical() == ip && sa.port() == from.port(),
        }
    }
}

/// A source whose configuration has been checked.
#[derive(Debug, Clone)]
pub struct Source {
    pub slot: String,
    pub listen: SocketAddr,
    pub interface: String,
    pub sender: Sender,
    pub cats: BTreeSet<u8>,
    pub volatile: bool,
}

impl Source {
    pub fn from_config(c: &SourceConfig) -> anyhow::Result<Self> {
        if !goat_provenance::cert::is_anchor(&c.slot) {
            anyhow::bail!("source slot {:?} is not a 32-hex slot anchor", c.slot);
        }
        let listen = c
            .listen
            .parse::<SocketAddr>()
            .map_err(|e| anyhow::anyhow!("source {}: listen {:?}: {e}", c.slot, c.listen))?;
        let sender =
            Sender::parse(&c.sender).map_err(|e| anyhow::anyhow!("source {}: {e}", c.slot))?;
        let cats = asterix::declared_cats(&c.format)
            .map_err(|e| anyhow::anyhow!("source {}: {e}", c.slot))?;
        if c.interface.trim().is_empty() {
            anyhow::bail!("source {}: interface is empty", c.slot);
        }
        Ok(Self {
            slot: c.slot.clone(),
            listen,
            interface: c.interface.clone(),
            sender,
            cats,
            volatile: c.volatile,
        })
    }

    /// The key a frame of category `cat` is published on.
    pub fn key_for(&self, cat: u8) -> String {
        self.key_with(&format!("{cat:03}"))
    }

    /// The key expression every frame of this source matches.
    pub fn key_pattern(&self) -> String {
        self.key_with("*")
    }

    fn key_with(&self, cat: &str) -> String {
        let base = format!("{}/stream/asterix/{cat}/raw/v1", self.slot);
        if self.volatile {
            format!("volatile/{base}")
        } else {
            base
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let c: Config =
            toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Ok(c)
    }

    /// Check every source; refuse an empty list and two sources on one port.
    pub fn sources(&self) -> anyhow::Result<Vec<Source>> {
        if self.sources.is_empty() {
            anyhow::bail!("no [[source]] declared");
        }
        let mut seen = HashMap::new();
        let mut out = Vec::new();
        for c in &self.sources {
            let s = Source::from_config(c)?;
            if let Some(other) = seen.insert(s.listen, s.slot.clone()) {
                anyhow::bail!("sources {other} and {} both listen on {}", s.slot, s.listen);
            }
            out.push(s);
        }
        Ok(out)
    }

    pub fn cert_path(&self) -> PathBuf {
        self.context.join("mtls.cert.pem")
    }
    pub fn key_path(&self) -> PathBuf {
        self.context.join("mtls.key.pem")
    }
    pub fn ca_roots_path(&self) -> PathBuf {
        self.context.join("ca-roots.pem")
    }

    /// The router to connect to: the override, else `router_endpoint` from
    /// the context's `profile.toml`.
    pub fn router_endpoint(&self) -> anyhow::Result<String> {
        if let Some(r) = &self.router {
            return Ok(r.clone());
        }
        let p = self.context.join("profile.toml");
        let text = std::fs::read_to_string(&p)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
        let v: toml::Value =
            toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
        find_string(&v, "router_endpoint")
            .ok_or_else(|| anyhow::anyhow!("{} has no router_endpoint", p.display()))
    }
}

fn find_string(v: &toml::Value, key: &str) -> Option<String> {
    match v {
        toml::Value::Table(t) => {
            if let Some(toml::Value::String(s)) = t.get(key) {
                return Some(s.clone());
            }
            t.values().find_map(|x| find_string(x, key))
        }
        _ => None,
    }
}

/// One sample ready to publish.
#[derive(Debug)]
pub struct Outgoing {
    pub key: String,
    pub payload: Vec<u8>,
    pub attachment: Vec<u8>,
}

/// Why a datagram was not published.
#[derive(Debug, PartialEq, Eq)]
pub enum Dropped {
    WrongSender(SocketAddr),
    Lint(String),
    Sign(String),
}

/// Turn one received datagram into the samples to publish.
///
/// Blocks are grouped by category in order of first appearance; each group is
/// one sample on that category's key. A datagram carrying one category (the
/// usual case) is published as received.
pub fn handle_datagram(
    src: &Source,
    signer: &Signer,
    from: SocketAddr,
    data: &[u8],
) -> Result<Vec<Outgoing>, Dropped> {
    if !src.sender.matches(from) {
        return Err(Dropped::WrongSender(from));
    }
    let groups = asterix::lint_and_group(data, &src.cats).map_err(Dropped::Lint)?;
    let via = Via {
        iface: src.interface.clone(),
        port: src.listen.port(),
        from: from.to_string(),
    };
    groups
        .into_iter()
        .map(|(cat, payload)| {
            let key = src.key_for(cat);
            let prov = sign(
                signer,
                SignedGrade::AdapterAttested,
                &src.slot,
                &key,
                &payload,
                Some(via.clone()),
            )
            .map_err(|e| Dropped::Sign(e.to_string()))?;
            let attachment = attach(None, &prov).map_err(|e| Dropped::Sign(e.to_string()))?;
            Ok(Outgoing {
                key,
                payload,
                attachment,
            })
        })
        .collect()
}

/// Counters for one source.
#[derive(Default, Debug)]
pub struct Stats {
    pub published: AtomicU64,
    pub wrong_sender: AtomicU64,
    pub lint_dropped: AtomicU64,
    pub failed: AtomicU64,
}

fn note(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

/// Log the 1st, 2nd, 4th, 8th… occurrence so a flood cannot fill the log.
fn worth_logging(n: u64) -> bool {
    n.is_power_of_two()
}

/// Receive, check, sign and publish for one source until `stop` fires.
pub async fn run_source(
    session: zenoh::Session,
    signer: Arc<Signer>,
    src: Source,
    sock: tokio::net::UdpSocket,
    stats: Arc<Stats>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut publishers: HashMap<String, zenoh::pubsub::Publisher<'static>> = HashMap::new();
    let mut buf = vec![0u8; 65535];
    loop {
        let (n, from) = tokio::select! {
            r = sock.recv_from(&mut buf) => r?,
            _ = stop.changed() => return Ok(()),
        };
        let out = match handle_datagram(&src, &signer, from, &buf[..n]) {
            Ok(out) => out,
            Err(Dropped::WrongSender(f)) => {
                let c = note(&stats.wrong_sender);
                if worth_logging(c) {
                    tracing::warn!(slot = %src.slot, from = %f, count = c, "dropped: not the declared sender");
                }
                continue;
            }
            Err(Dropped::Lint(why)) => {
                let c = note(&stats.lint_dropped);
                if worth_logging(c) {
                    tracing::warn!(slot = %src.slot, len = n, count = c, "dropped, not published: {why}");
                }
                continue;
            }
            Err(Dropped::Sign(why)) => {
                note(&stats.failed);
                tracing::error!(slot = %src.slot, "could not sign: {why}");
                continue;
            }
        };
        for o in out {
            if !publishers.contains_key(&o.key) {
                let p = session
                    .declare_publisher(o.key.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("declare publisher {}: {e}", o.key))?;
                publishers.insert(o.key.clone(), p);
            }
            let p = &publishers[&o.key];
            match p.put(o.payload).attachment(o.attachment).await {
                Ok(()) => {
                    note(&stats.published);
                }
                Err(e) => {
                    let c = note(&stats.failed);
                    if worth_logging(c) {
                        tracing::error!(key = %o.key, count = c, "publish failed: {e}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_matching() {
        let only_ip = Sender::parse("192.0.2.10").unwrap();
        assert!(only_ip.matches("192.0.2.10:5000".parse().unwrap()));
        assert!(only_ip.matches("[::ffff:192.0.2.10]:5000".parse().unwrap()));
        assert!(!only_ip.matches("192.0.2.11:5000".parse().unwrap()));
        let ip_port = Sender::parse("192.0.2.10:5000").unwrap();
        assert!(ip_port.matches("192.0.2.10:5000".parse().unwrap()));
        assert!(!ip_port.matches("192.0.2.10:5001".parse().unwrap()));
        assert!(Sender::parse("radar.example").is_err());
    }

    #[test]
    fn keys_carry_the_source_slot_and_the_category() {
        let c = SourceConfig {
            slot: "00112233445566778899aabbccddeeff".into(),
            listen: "0.0.0.0:30048".into(),
            interface: "udp".into(),
            sender: "192.0.2.10".into(),
            format: "asterix:034,048".into(),
            volatile: false,
        };
        let s = Source::from_config(&c).unwrap();
        assert_eq!(
            s.key_for(48),
            "00112233445566778899aabbccddeeff/stream/asterix/048/raw/v1"
        );
        let v = Source::from_config(&SourceConfig {
            volatile: true,
            ..c.clone()
        })
        .unwrap();
        assert_eq!(
            v.key_for(34),
            "volatile/00112233445566778899aabbccddeeff/stream/asterix/034/raw/v1"
        );
        assert!(Source::from_config(&SourceConfig {
            slot: "radar-1".into(),
            ..c
        })
        .is_err());
    }

    #[test]
    fn config_refuses_two_sources_on_one_port() {
        let text = r#"
context = "/etc/goat-ingress/context"
[[source]]
slot = "00112233445566778899aabbccddeeff"
listen = "0.0.0.0:30048"
sender = "192.0.2.10"
[[source]]
slot = "ffeeddccbbaa99887766554433221100"
listen = "0.0.0.0:30048"
sender = "192.0.2.11"
"#;
        let c: Config = toml::from_str(text).unwrap();
        assert!(c.sources().unwrap_err().to_string().contains("both listen"));
    }

    #[test]
    fn router_comes_from_the_context_profile() {
        let dir = std::env::temp_dir().join(format!("goat-ingress-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("profile.toml"),
            "[zenoh]\nrouter_endpoint = \"tls/203.0.113.5:7447\"\n",
        )
        .unwrap();
        let c = Config {
            context: dir.clone(),
            router: None,
            sources: vec![],
        };
        assert_eq!(c.router_endpoint().unwrap(), "tls/203.0.113.5:7447");
        let o = Config {
            router: Some("tls/198.51.100.1:7447".into()),
            ..c
        };
        assert_eq!(o.router_endpoint().unwrap(), "tls/198.51.100.1:7447");
        let _ = std::fs::remove_dir_all(dir);
    }
}
