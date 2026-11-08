//! Block 33C.1 lever L8.b — SO_REUSEPORT_CBPF source-IP affinity steering.
//!
//! Default `SO_REUSEPORT` distributes new connections across shards using
//! the kernel's RSS hash + a per-CPU pseudo-random factor. This is correct
//! for connection-level load balancing but spreads traffic from the same
//! source IP across multiple shards. For a relay, that means peer A's
//! bidirectional traffic can land on a different shard than where peer A's
//! connection state lives — forcing cross-shard MPSC synchronization on
//! every forwarded message.
//!
//! L8.b attaches a classic BPF program to the SO_REUSEPORT group via
//! `SO_ATTACH_REUSEPORT_CBPF`. The program reads the IPv4 source IP from
//! the incoming packet and returns `src_ip % num_sockets`, so the same
//! peer always lands on the same shard — keeping that shard's routing
//! table cache-hot and eliminating the cross-shard MPSC sync for normal
//! steady-state traffic.
//!
//! Expected lift per the perf-hardening doc: +10-25% on paired-flow
//! workloads. Single-flow workloads see no benefit (only one shard is
//! ever active). M-019's break-even verdict on single-flow + M-020's
//! ~2× single-to-aggregate scaling suggest L8.b's win shows up at
//! ≥4 concurrent peer pairs.
//!
//! ## Attach semantics
//!
//! `SO_ATTACH_REUSEPORT_CBPF` applies group-wide: attaching on any one
//! socket in the SO_REUSEPORT group affects steering for the whole group.
//! We attach on the first shard's socket; subsequent shards inherit the
//! steering rule via the kernel-managed group state.
//!
//! Detaching is automatic when the group's last socket closes; explicit
//! detach via `SO_DETACH_REUSEPORT_BPF` is available but not used here.
//!
//! ## BPF program details
//!
//! Three classic-BPF instructions:
//! ```text
//! ld   [SKF_NET_OFF + 12] ; load 32-bit word at network-header offset 12 (IPv4 src IP)
//! mod  num_sockets        ; modulo num_sockets — returns shard index 0..N-1
//! ret  A                  ; return accumulator (the result)
//! ```
//!
//! The "packet" a reuseport program sees does NOT start at the IP header:
//! the kernel pulls the transport header first, so plain offset 0 is the
//! first byte after the TCP/UDP header (the TCP payload, empty on a SYN).
//! A plain `ld [12]` therefore reads past the end of a SYN, the load
//! aborts the program with a return of 0, and every connection lands on
//! socket 0. The address has to be read relative to the network header,
//! through the `SKF_NET_OFF` ancillary base.
//!
//! For IPv4 the load reads the source address (header bytes 12-15). For
//! an IPv6 packet the same load reads bytes 4-7 of the 16-byte source
//! address, which still pins one source to one shard.
//!
//! Linux 4.5+ required (SO_ATTACH_REUSEPORT_CBPF added in commit 538950a1b75aa).

use std::io;
use std::os::fd::AsRawFd;

/// Attach a SO_REUSEPORT_CBPF program that steers IPv4 connections to
/// `src_ip % num_sockets` socket within the SO_REUSEPORT group.
///
/// Call this on ONE socket after `SO_REUSEPORT` is set + `bind()` is done.
/// The program applies to the whole group via Linux's reuseport group
/// state; subsequent sockets joining the group inherit the steering.
///
/// # Errors
///
/// Returns `io::Error` if the underlying `setsockopt` call fails. Common
/// causes: kernel < 4.5, `SO_REUSEPORT` not set on the socket, or the BPF
/// program is rejected by the kernel verifier (shouldn't happen for this
/// 3-instruction program).
pub(crate) fn attach_source_ip_steering<S: AsRawFd>(
    socket: &S,
    num_sockets: u32,
) -> io::Result<()> {
    if num_sockets == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "num_sockets must be > 0",
        ));
    }

    // Classic BPF (cBPF) instruction encoding:
    //   sock_filter { code: u16, jt: u8, jf: u8, k: u32 }
    //
    // BPF opcodes (from <linux/bpf_common.h>):
    //   BPF_LD = 0x00, BPF_W = 0x00, BPF_ABS = 0x20  → 0x20 (load 32-bit absolute)
    //   BPF_ALU = 0x04, BPF_MOD = 0x90, BPF_K = 0x00 → 0x94 (mod-A-with-K)
    //   BPF_RET = 0x06, BPF_A = 0x10                  → 0x16 (return accumulator)
    //
    // Constants encoded directly to avoid pulling in extra crates for
    // BPF instruction builders. The 3-instruction program is small enough
    // to verify by inspection.
    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_ALU_MOD_K: u16 = 0x94;
    const BPF_RET_A: u16 = 0x16;

    // IPv4 source IP is at offset 12 in the IP header. A reuseport
    // program's packet pointer sits past the transport header, so the
    // offset is taken from the network header via the SKF_NET_OFF base
    // (see the module docs). A plain 12 reads past the end of a SYN and
    // steers every connection to socket 0.
    #[allow(clippy::cast_sign_loss)]
    const IPV4_SRC_OFFSET: u32 = (libc::SKF_NET_OFF + 12) as u32;

    let prog: [libc::sock_filter; 3] = [
        libc::sock_filter {
            code: BPF_LD_W_ABS,
            jt: 0,
            jf: 0,
            k: IPV4_SRC_OFFSET,
        },
        libc::sock_filter {
            code: BPF_ALU_MOD_K,
            jt: 0,
            jf: 0,
            k: num_sockets,
        },
        libc::sock_filter {
            code: BPF_RET_A,
            jt: 0,
            jf: 0,
            k: 0,
        },
    ];

    let fprog = libc::sock_fprog {
        len: prog.len() as u16,
        // sock_fprog.filter is *mut sock_filter — the kernel reads from
        // this pointer during the setsockopt call but does not retain it.
        // Cast away const here is correct: the array lives on this stack
        // frame for the duration of the syscall.
        filter: prog.as_ptr() as *mut libc::sock_filter,
    };

    let fd = socket.as_raw_fd();
    // SAFETY: setsockopt is FFI; the workspace denies `unsafe_code` by
    // default. This block is the entire unsafe surface of L8.b — calling
    // setsockopt(2) with SO_ATTACH_REUSEPORT_CBPF + a sock_fprog living
    // on this stack frame for the duration of the call. The kernel reads
    // from `fprog.filter` synchronously during the syscall and does not
    // retain the pointer afterward (the program is copied into kernel
    // memory). The fd ownership is borrowed via `socket.as_raw_fd()`;
    // we do not close or otherwise mutate the socket. Allowed at this
    // single call site only.
    #[allow(unsafe_code)]
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ATTACH_REUSEPORT_CBPF,
            &fprog as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::sock_fprog>() as libc::socklen_t,
        )
    };

    if ret < 0 {
        let err = io::Error::last_os_error();
        eprintln!(
            "L8.b CBPF attach failed on fd={fd}: {err} (kernel < 4.5? SO_REUSEPORT not set first?)"
        );
        Err(err)
    } else {
        eprintln!(
            "L8.b CBPF attached on fd={fd}: src_ip % {num_sockets} steering active group-wide"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    /// Constructs a TCP socket, sets SO_REUSEPORT, and verifies that the
    /// CBPF attach call succeeds.
    ///
    /// Skipped on non-Linux (the module is `#[cfg(target_os = "linux")]`)
    /// — the test runs only on Linux CI.
    #[test]
    fn attach_succeeds_on_reuseport_socket() {
        use socket2::{Domain, Socket, Type};

        let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
        socket.set_reuse_port(true).unwrap();

        // bind to an ephemeral port on localhost — required for the
        // socket to be in the SO_REUSEPORT group state that CBPF can
        // attach to.
        let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        socket.bind(&addr.into()).unwrap();

        let fd = socket.as_raw_fd();
        // Wrapper that just exposes the fd; Socket implements AsRawFd already.
        struct FdHolder(i32);
        impl AsRawFd for FdHolder {
            fn as_raw_fd(&self) -> i32 {
                self.0
            }
        }

        // 4 sockets is a typical small shard count; the modulo arithmetic
        // is the same regardless.
        attach_source_ip_steering(&FdHolder(fd), 4).expect("CBPF attach");
    }

    #[test]
    fn rejects_zero_sockets() {
        // A zero-shard program would divide by zero in the kernel BPF
        // execution, so reject at the API level.
        struct DummyFd;
        impl AsRawFd for DummyFd {
            fn as_raw_fd(&self) -> i32 {
                -1
            }
        }
        let err = attach_source_ip_steering(&DummyFd, 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    /// End-to-end behavior test: open 4 reuseport sockets, attach the CBPF
    /// program, fire connections from synthetic source IPs, verify each
    /// connection lands on the expected socket per `src_ip % 4`.
    ///
    /// Why this test matters: the constructor + attach test only verify
    /// the syscall succeeds. This test verifies the BPF program semantics
    /// — that the kernel actually runs `src_ip % N` and steers traffic
    /// to the right socket index. A misencoded BPF instruction would
    /// pass the attach call but route incorrectly; this test catches
    /// that class of bug.
    ///
    /// Linux-only by virtue of the `#[cfg(target_os = "linux")]` on the
    /// module. Skipped on other OSes; runs in CI on Linux.
    #[test]
    fn end_to_end_steering_pins_same_src_to_same_socket() {
        use socket2::{Domain, Socket, Type};
        use std::net::{TcpStream, SocketAddr};

        const N: u32 = 4;

        // Open N listening sockets on the same port via SO_REUSEPORT.
        // After bind, attach CBPF on the FIRST socket (group-wide).
        let mut sockets = Vec::with_capacity(N as usize);
        for _ in 0..N {
            let s = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
            s.set_reuse_address(true).unwrap();
            s.set_reuse_port(true).unwrap();
            // bind to ephemeral port on first iteration; subsequent sockets
            // bind to the same address by reading the first's local_addr.
            sockets.push(s);
        }
        // Bind the first to ephemeral port, then bind the others to the
        // same address.
        let zero_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        sockets[0].bind(&zero_addr.into()).unwrap();
        let bound: SocketAddr = sockets[0].local_addr().unwrap().as_socket().unwrap();
        for s in sockets.iter().skip(1) {
            s.bind(&bound.into()).unwrap();
        }

        // Attach CBPF on socket 0 — group-wide effect.
        attach_source_ip_steering(&sockets[0], N).expect("CBPF attach");

        // Listen on all of them.
        for s in &sockets {
            s.listen(16).unwrap();
        }

        // Fire a TCP connection from 127.0.0.1 (single source IP — kernel
        // RSS would distribute randomly without CBPF, but with CBPF the
        // src IP `127.0.0.1` = 0x7F000001 mod 4 = 1, so socket index 1
        // should accept all connections from this source.
        //
        // Note: this test connects from the same source IP (all loopback),
        // so all connections deterministically route to socket
        // `0x7F000001 % 4 = 1`. We fire 4 connections; if the BPF program
        // is wrong, distribution would be RSS-random (statistically
        // unlikely all 4 land on the same wrong socket).
        sockets[1]
            .set_nonblocking(true)
            .expect("set_nonblocking on expected socket");

        let mut connections_landed_on_1 = 0;
        for _ in 0..4 {
            let _stream = TcpStream::connect(bound).expect("connect");
            // Try to accept on socket 1 (the expected destination).
            // Brief retry loop — kernel may take a moment to enqueue.
            for _ in 0..50 {
                match sockets[1].accept() {
                    Ok(_) => {
                        connections_landed_on_1 += 1;
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => panic!("unexpected accept error: {e}"),
                }
            }
        }

        // All 4 connections from src=127.0.0.1 (mod 4 = 1) MUST land on
        // socket 1. If even one lands elsewhere, the BPF program is wrong
        // OR the kernel didn't honor the steering.
        assert_eq!(
            connections_landed_on_1, 4,
            "CBPF should pin all connections from src=127.0.0.1 (mod 4 = 1) to socket 1; \
             only {connections_landed_on_1}/4 landed there"
        );
    }
}
