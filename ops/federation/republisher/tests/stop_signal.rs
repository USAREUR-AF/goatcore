// SPDX-License-Identifier: Apache-2.0
//
// The republisher stops on SIGTERM, which is what `docker stop` and a compose
// restart send. It runs as PID 1 in its container with no init, and PID 1
// ignores a signal it has no handler for, so without a handler the stop waits
// out docker's grace period and ends in SIGKILL: the hand-off queue is never
// drained and the publishing session is never closed.
//
// These tests run the real binary against a router opened in this process.
// They are not PID 1, so a missing handler shows here as death by signal
// rather than as a hang.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_goat-fed-republisher");
const US: &str = "xaa";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The compose-full vector with its expiry moved out of the way and the
/// ceilings raised, so neither the date nor the envelope decides these tests.
fn agreement(dir: &str) -> PathBuf {
    let raw = include_str!("../../vectors/compose-full/agreement.pretty.json");
    let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
    v["expires"] = "2099-12-31".into();
    for f in v["flows"].as_array_mut().unwrap() {
        f["envelope"]["hz"] = 1_000_000.into();
        f["envelope"]["distinct_keys"] = 1_000_000.into();
    }
    let d = std::env::temp_dir().join(format!("rp-stop-{dir}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("agreement.json");
    std::fs::write(&p, serde_json::to_string(&v).unwrap()).unwrap();
    p
}

struct Running {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Running {
    fn start(agreement: &Path, endpoint: &str) -> Self {
        let mut child = Command::new(BIN)
            .env_clear()
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .args(["--agreement", agreement.to_str().unwrap()])
            .args(["--sovereign-id", US])
            .args(["--local-endpoint", endpoint])
            .args(["--edge-endpoint", endpoint])
            .args(["--metrics-bind", "127.0.0.1:0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the republisher");
        let lines = Arc::new(Mutex::new(Vec::new()));
        for out in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let lines = lines.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    lines.lock().unwrap().push(l);
                }
            });
        }
        Running { child, lines }
    }

    fn log(&self) -> String {
        self.lines.lock().unwrap().join("\n")
    }

    fn wait_for_line(&self, needle: &str, within: Duration) {
        let end = Instant::now() + within;
        while Instant::now() < end {
            if self.log().contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "no {needle:?} in the log within {within:?}:\n{}",
            self.log()
        );
    }

    fn sigterm(&self) {
        let ok = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill -TERM failed");
    }

    /// The exit status, or a kill and a panic if it is still running after
    /// `within`: a stop that outlives docker's grace is the failure under test.
    fn exit_within(&mut self, within: Duration) -> ExitStatus {
        let end = Instant::now() + within;
        while Instant::now() < end {
            if let Some(s) = self.child.try_wait().unwrap() {
                // Let the reader threads take the last lines.
                std::thread::sleep(Duration::from_millis(100));
                return s;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        panic!("still running {within:?} after SIGTERM:\n{}", self.log());
    }
}

fn assert_clean_exit(status: ExitStatus, log: &str) {
    use std::os::unix::process::ExitStatusExt;
    assert!(
        status.signal().is_none(),
        "killed by signal {:?}: SIGTERM has no handler\n{log}",
        status.signal()
    );
    assert!(status.success(), "exit {status}\n{log}");
}

/// The stop counts the final line prints: `stopped: received R, published P, dropped D`.
fn stop_counts(log: &str) -> (u64, u64, u64) {
    let l = log
        .lines()
        .find_map(|l| {
            l.split_once("stopped: received ")
                .map(|(_, r)| r.to_string())
        })
        .unwrap_or_else(|| panic!("no stop line in the log:\n{log}"));
    let n: Vec<u64> = l
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(n.len(), 3, "stop line {l:?}");
    (n[0], n[1], n[2])
}

#[test]
fn sigterm_while_the_core_is_unreachable_exits_cleanly() {
    let ag = agreement("unreachable");
    // Nobody listens here, so the process is still opening its sessions.
    let endpoint = format!("tcp/127.0.0.1:{}", free_port());
    let mut rp = Running::start(&ag, &endpoint);
    rp.wait_for_line("agreement loaded", Duration::from_secs(20));
    std::thread::sleep(Duration::from_millis(500));
    rp.sigterm();
    let status = rp.exit_within(Duration::from_secs(8));
    let log = rp.log();
    assert_clean_exit(status, &log);
    assert!(log.contains("SIGTERM"), "the stop names its signal:\n{log}");
    let _ = std::fs::remove_dir_all(ag.parent().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_drains_what_was_received_closes_the_session_and_exits_cleanly() {
    let port = free_port();
    let endpoint = format!("tcp/127.0.0.1:{port}");
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    let router = zenoh::open(c).await.unwrap();

    let arrived = Arc::new(AtomicUsize::new(0));
    let counter = arrived.clone();
    let _sub = router
        .declare_subscriber(format!("from/{US}/**"))
        .callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();

    let ag = agreement("drain");
    let mut rp = Running::start(&ag, &endpoint);
    rp.wait_for_line("republishing", Duration::from_secs(30));
    // The subscription has to reach the router before the burst does.
    tokio::time::sleep(Duration::from_millis(500)).await;

    for i in 0..300 {
        router
            .put(format!("release/weather/t/{i}"), format!("v{i}"))
            .await
            .unwrap();
    }
    // At once, so samples are still in the subscriber's FIFO and the hand-off
    // queue when the signal lands.
    rp.sigterm();
    // Blocks this worker; the router runs on zenoh's own threads.
    let status = rp.exit_within(Duration::from_secs(8));
    let log = rp.log();
    assert_clean_exit(status, &log);
    assert!(log.contains("SIGTERM"), "the stop names its signal:\n{log}");

    let (received, published, dropped) = stop_counts(&log);
    eprintln!("stop: received {received}, published {published}, dropped {dropped}");
    assert!(received > 0, "the burst reached the republisher:\n{log}");
    assert_eq!(dropped, 0, "nothing dropped at the stop:\n{log}");
    assert_eq!(
        published, received,
        "everything received before the stop was published:\n{log}"
    );
    // What it published reached the router: the session was closed, not dropped.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        arrived.load(Ordering::SeqCst) as u64,
        published,
        "every published sample arrived at the router:\n{log}"
    );
    let _ = std::fs::remove_dir_all(ag.parent().unwrap());
}
