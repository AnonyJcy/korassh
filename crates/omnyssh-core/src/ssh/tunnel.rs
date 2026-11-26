//! Local port forwarding — the `ssh -L` / `LocalForward` model.
//!
//! A host's tunnel is one SSH connection carrying every [`LocalForward`] the
//! host defines. Each forward listens on a local port, and every connection it
//! accepts rides its own `direct-tcpip` channel to `remote_host:remote_port`,
//! which the server resolves — so `localhost` there is the server's own loopback.
//!
//! [`TunnelManager`] runs one task per host. The local ports are bound before the
//! first dial and held until the tunnel stops, so a dropped connection never
//! hands them to another process: the task reconnects with a backoff, and
//! connections that arrive meanwhile wait in the listen backlog. Every change is
//! reported as [`CoreEvent::TunnelStatusChanged`].

use std::collections::HashMap;
use std::fmt;
use std::future::poll_fn;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;

use crate::event::CoreEvent;
use crate::ssh::client::Host;
use crate::ssh::identity;
use crate::ssh::password;
use crate::ssh::session::{
    connect_and_auth, connect_budget, is_refused, passphrase_required, waiting_login, Passwords,
    SshConnection,
};

// ---------------------------------------------------------------------------
// LocalForward
// ---------------------------------------------------------------------------

/// One `ssh -L` rule: `[bind_address:]port:host:hostport`.
///
/// Stored in `hosts.toml` in that same notation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub struct LocalForward {
    /// Local address to listen on. `None` and `localhost` mean the loopback
    /// only, as with ssh; `*` or an empty address mean every interface.
    pub bind_address: Option<String>,
    /// Local port to listen on.
    pub bind_port: u16,
    /// Where the server connects to, resolved on the server's side.
    pub remote_host: String,
    /// The port the server connects to.
    pub remote_port: u16,
}

impl FromStr for LocalForward {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let spec = spec.trim();
        let usage = || format!("expected [bind_address:]port:host:hostport, got '{spec}'");
        let fields = split_fields(spec).ok_or_else(usage)?;
        let (bind_address, bind_port, remote_host, remote_port) = match fields.as_slice() {
            [port, host, hostport] => (None, *port, *host, *hostport),
            [bind, port, host, hostport] => (Some(*bind), *port, *host, *hostport),
            _ => return Err(usage()),
        };
        if remote_host.is_empty() {
            return Err(usage());
        }
        Ok(Self {
            bind_address: bind_address.map(str::to_string),
            bind_port: parse_port(bind_port, spec)?,
            remote_host: remote_host.to_string(),
            remote_port: parse_port(remote_port, spec)?,
        })
    }
}

impl fmt::Display for LocalForward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(bind) = &self.bind_address {
            write!(f, "{}:", bracketed(bind))?;
        }
        write!(
            f,
            "{}:{}:{}",
            self.bind_port,
            bracketed(&self.remote_host),
            self.remote_port
        )
    }
}

impl From<LocalForward> for String {
    fn from(forward: LocalForward) -> Self {
        forward.to_string()
    }
}

// ---------------------------------------------------------------------------
// RemoteForward (`ssh -R`)
// ---------------------------------------------------------------------------

/// One `ssh -R` rule: `[bind_address:]remote_port:local_host:local_port`.
///
/// The SSH server listens on `bind_address:remote_port` and forwards incoming
/// connections back over `forwarded-tcpip` to `local_host:local_port` here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub struct RemoteForward {
    pub bind_address: Option<String>,
    pub remote_port: u16,
    pub local_host: String,
    pub local_port: u16,
}

impl FromStr for RemoteForward {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let spec = spec.trim();
        let usage =
            || format!("expected [bind_address:]remote_port:local_host:local_port, got '{spec}'");
        let fields = split_fields(spec).ok_or_else(usage)?;
        let (bind_address, remote_port, local_host, local_port) = match fields.as_slice() {
            [rport, host, lport] => (None, *rport, *host, *lport),
            [bind, rport, host, lport] => (Some(*bind), *rport, *host, *lport),
            _ => return Err(usage()),
        };
        if local_host.is_empty() {
            return Err(usage());
        }
        Ok(Self {
            bind_address: bind_address.map(str::to_string),
            remote_port: parse_port(remote_port, spec)?,
            local_host: local_host.to_string(),
            local_port: parse_port(local_port, spec)?,
        })
    }
}

impl fmt::Display for RemoteForward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(bind) = &self.bind_address {
            write!(f, "{}:", bracketed(bind))?;
        }
        write!(
            f,
            "{}:{}:{}",
            self.remote_port,
            bracketed(&self.local_host),
            self.local_port
        )
    }
}

impl From<RemoteForward> for String {
    fn from(forward: RemoteForward) -> Self {
        forward.to_string()
    }
}

// ---------------------------------------------------------------------------
// DynamicForward (`ssh -D`, SOCKS5)
// ---------------------------------------------------------------------------

/// One `ssh -D` rule: `[bind_address:]bind_port`.
///
/// Opens a local SOCKS5 proxy (RFC 1928) that routes every client `CONNECT`
/// request over a `direct-tcpip` SSH channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub struct DynamicForward {
    pub bind_address: Option<String>,
    pub bind_port: u16,
}

impl FromStr for DynamicForward {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let spec = spec.trim();
        let usage = || format!("expected [bind_address:]bind_port, got '{spec}'");
        let fields = split_fields(spec).ok_or_else(usage)?;
        let (bind_address, bind_port) = match fields.as_slice() {
            [port] => (None, *port),
            [bind, port] => (Some(*bind), *port),
            _ => return Err(usage()),
        };
        Ok(Self {
            bind_address: bind_address.map(str::to_string),
            bind_port: parse_port(bind_port, spec)?,
        })
    }
}

impl fmt::Display for DynamicForward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(bind) = &self.bind_address {
            write!(f, "{}:", bracketed(bind))?;
        }
        write!(f, "{}", self.bind_port)
    }
}

impl From<DynamicForward> for String {
    fn from(forward: DynamicForward) -> Self {
        forward.to_string()
    }
}

/// Splits on the colons outside `[...]`, unwrapping a bracketed IPv6 address.
/// `None` when a bracket is left open.
fn split_fields(spec: &str) -> Option<Vec<&str>> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut in_brackets = false;
    for (i, c) in spec.char_indices() {
        match c {
            '[' => in_brackets = true,
            ']' => in_brackets = false,
            ':' if !in_brackets => {
                fields.push(&spec[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if in_brackets {
        return None;
    }
    fields.push(&spec[start..]);
    Some(
        fields
            .into_iter()
            .map(|f| {
                f.strip_prefix('[')
                    .and_then(|f| f.strip_suffix(']'))
                    .unwrap_or(f)
            })
            .collect(),
    )
}

fn parse_port(value: &str, spec: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|&p| p != 0)
        .ok_or_else(|| format!("'{value}' is not a port between 1 and 65535 in '{spec}'"))
}

/// An IPv6 address needs brackets to survive the colon-separated notation.
fn bracketed(address: &str) -> String {
    if address.contains(':') {
        format!("[{address}]")
    } else {
        address.to_string()
    }
}

// ---------------------------------------------------------------------------
// TunnelStatus
// ---------------------------------------------------------------------------

/// Where a host's tunnel stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    /// Dialling the host: the first attempt, or the next one after a drop.
    Connecting,
    /// Connected, with every forward listening.
    Up,
    /// The connection failed or dropped; another attempt follows a backoff.
    /// The ports stay bound in the meantime.
    Retrying(String),
    /// Ended and will not retry: a port could not be bound, or the server
    /// refused the credentials or its host key.
    Failed(String),
    /// Stopped on request.
    Stopped,
}

// ---------------------------------------------------------------------------
// TunnelManager
// ---------------------------------------------------------------------------

/// How long after a failed or dropped connection the next dial waits. The last
/// step repeats.
const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// A connection that lasted this long resets the backoff. Resetting on connect
/// alone would redial every second a server that accepts and then drops us.
const STABLE_AFTER: Duration = Duration::from_secs(60);

/// Head room over [`connect_budget`] for the password steps of a login.
const AUTH_BUDGET: Duration = Duration::from_secs(20);

/// How often a tunnel with no key that gets in and no password tries again.
const NO_PASSWORD_RETRY: Duration = Duration::from_secs(60);

/// How often a live tunnel checks that its connection still is. A dead peer
/// is noticed by the keepalives first; this only picks that up.
const LIVENESS_CHECK: Duration = Duration::from_secs(1);

/// Runs the tunnels of every host, one task each.
///
/// Dropping the manager stops them all.
pub struct TunnelManager {
    tx: mpsc::Sender<CoreEvent>,
    runs: HashMap<String, Run>,
}

/// One host's tunnel task. A stopped run stays listed until it has wound down,
/// so a start right after it still waits for the ports it holds.
struct Run {
    /// The host as the tunnel was started, to tell a later edit apart.
    host: Host,
    /// Dropped or fired, it ends the task; `None` once it has been asked to.
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Run {
    /// Asks the task to end; it reports [`TunnelStatus::Stopped`] on its way out.
    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl TunnelManager {
    /// Creates a manager that reports on `tx`.
    pub fn new(tx: mpsc::Sender<CoreEvent>) -> Self {
        Self {
            tx,
            runs: HashMap::new(),
        }
    }

    /// Starts `host`'s tunnel, or restarts it when one is already running. The
    /// new run waits for the old one to release the ports before binding them.
    ///
    /// Must be called within a tokio runtime.
    pub fn start(&mut self, host: Host) {
        let previous = self.runs.remove(&host.name).map(|mut run| {
            run.stop();
            run.task
        });
        let (stop, stop_rx) = oneshot::channel();
        let task = tokio::spawn(run_tunnel(host.clone(), self.tx.clone(), stop_rx, previous));
        let stop = Some(stop);
        self.runs
            .insert(host.name.clone(), Run { host, stop, task });
    }

    /// Starts the tunnel of every host marked to start on launch.
    pub fn autostart(&mut self, hosts: &[Host]) {
        for host in hosts {
            if host.tunnel_autostart && host.has_tunnels() {
                self.start(host.clone());
            }
        }
    }

    /// Stops `name`'s tunnel. Does nothing when there is none.
    pub fn stop(&mut self, name: &str) {
        if let Some(run) = self.runs.get_mut(name) {
            run.stop();
        }
    }

    /// Whether `name` has a tunnel that has neither ended nor been stopped.
    pub fn is_running(&self, name: &str) -> bool {
        self.runs
            .get(name)
            .is_some_and(|run| run.stop.is_some() && !run.task.is_finished())
    }

    /// Brings the running tunnels in line with an edited host list: a host that
    /// is gone, or has no forwards left, is stopped, and one whose connection or
    /// forwards changed is restarted. Nothing is started that was not running.
    pub fn sync(&mut self, hosts: &[Host]) {
        self.runs.retain(|_, run| !run.task.is_finished());
        let names: Vec<String> = self.runs.keys().cloned().collect();
        for name in names {
            if !self.is_running(&name) {
                continue;
            }
            match hosts.iter().find(|h| h.name == name) {
                Some(host) if !host.has_tunnels() => self.stop(&name),
                Some(host) if self.runs.get(&name).is_some_and(|r| changed(&r.host, host)) => {
                    self.start(host.clone());
                }
                Some(_) => {}
                None => self.stop(&name),
            }
        }
    }

    /// Stops every tunnel.
    pub fn shutdown(mut self) {
        for run in self.runs.values_mut() {
            run.stop();
        }
    }
}

/// Whether an edit touched anything a running tunnel was started with.
fn changed(before: &Host, after: &Host) -> bool {
    before.hostname != after.hostname
        || before.port != after.port
        || before.user != after.user
        || before.identity_file != after.identity_file
        || before.identities_only != after.identities_only
        || before.password != after.password
        || before.proxy_jump != after.proxy_jump
        || before.proxy != after.proxy
        || before.local_forwards != after.local_forwards
        || before.remote_forwards != after.remote_forwards
        || before.dynamic_forwards != after.dynamic_forwards
}

// ---------------------------------------------------------------------------
// Tunnel task
// ---------------------------------------------------------------------------

async fn run_tunnel(
    host: Host,
    tx: mpsc::Sender<CoreEvent>,
    mut stop: oneshot::Receiver<()>,
    previous: Option<JoinHandle<()>>,
) {
    // A restart must not race its predecessor for the ports — even when it is
    // stopped first, or the next run would inherit that race. The predecessor has
    // been told to stop, so this is short.
    if let Some(previous) = previous {
        let _ = previous.await;
    }
    let status = tokio::select! {
        biased;
        _ = &mut stop => TunnelStatus::Stopped,
        status = serve(&host, &tx) => status,
    };
    // `serve` is dropped by now, so the ports are free again.
    send_status(&tx, &host.name, status).await;
}

/// Holds the ports and keeps the connection up. Returns only once the tunnel
/// cannot go on.
async fn serve(host: &Host, tx: &mpsc::Sender<CoreEvent>) -> TunnelStatus {
    if !host.has_tunnels() {
        return TunnelStatus::Failed(String::from("no port forwards are set up for this host"));
    }
    let listeners = match bind_all(&host.local_forwards, &host.dynamic_forwards).await {
        Ok(listeners) => listeners,
        Err(e) => return TunnelStatus::Failed(e),
    };

    // Only the first dial reports Connecting; a retry keeps showing why the last
    // attempt failed until one succeeds.
    send_status(tx, &host.name, TunnelStatus::Connecting).await;
    let mut retry = 0;
    loop {
        let budget = connect_budget(host).await + AUTH_BUDGET;
        let mut locked = None;
        let mut waiting = None;
        let reason =
            match time::timeout(budget, connect_and_auth(host, Passwords::Remembered)).await {
                Ok(Ok(conn)) => {
                    let conn = Arc::new(conn);
                    match setup_remote_forwards(&conn, &host.remote_forwards).await {
                        Ok(()) => {
                            send_status(tx, &host.name, TunnelStatus::Up).await;
                            let since = Instant::now();
                            let reason = forward(conn, &listeners, tx, &host.name).await;
                            if since.elapsed() >= STABLE_AFTER {
                                retry = 0;
                            }
                            reason
                        }
                        Err(e) => e,
                    }
                }
                Ok(Err(e)) if is_refused(&e) => return TunnelStatus::Failed(format!("{e:#}")),
                Ok(Err(e)) => {
                    locked = passphrase_required(&e).map(str::to_owned);
                    waiting = waiting_login(&e).map(str::to_owned);
                    format!("{e:#}")
                }
                Err(_) => format!(
                    "no answer from {} within {}s",
                    host.hostname,
                    budget.as_secs()
                ),
            };
        tracing::debug!(host = %host.name, %reason, "tunnel down");
        send_status(tx, &host.name, TunnelStatus::Retrying(reason)).await;
        if let Some(login) = waiting {
            // Redialling cannot help until the key is unlocked or a password is
            // typed for the login (in a terminal, say), and every try is a failed
            // login on the server: hold the ports and wait for either.
            if let Some(path) = locked {
                identity::ask_passphrase_once(tx, &host.name, &path).await;
                tokio::select! {
                    () = identity::unlocked(&path) => {}
                    () = password::remembered(&login) => {}
                }
            } else {
                // No key got in, which an agent unlocked meanwhile could change:
                // look again now and then.
                tokio::select! {
                    () = password::remembered(&login) => {}
                    () = time::sleep(NO_PASSWORD_RETRY) => {}
                }
            }
            continue;
        }
        time::sleep(RETRY_DELAYS[retry]).await;
        retry = (retry + 1).min(RETRY_DELAYS.len() - 1);
    }
}

/// Requests `tcpip-forward` on the server for each `ssh -R` rule and registers
/// its local target route on `conn`.
async fn setup_remote_forwards(
    conn: &SshConnection,
    forwards: &[RemoteForward],
) -> Result<(), String> {
    for rule in forwards {
        let bind_addr = match rule.bind_address.as_deref() {
            None | Some("localhost") => "127.0.0.1".to_string(),
            Some("" | "*") => "0.0.0.0".to_string(),
            Some(addr) => addr.to_string(),
        };
        let bound_port = conn
            .tcpip_forward(bind_addr.clone(), u32::from(rule.remote_port))
            .await
            .map_err(|e| format!("remote forward {rule} rejected by server: {e}"))?;
        let actual_port = if bound_port != 0 {
            bound_port
        } else {
            u32::from(rule.remote_port)
        };
        conn.register_remote_forward(
            bind_addr,
            actual_port,
            rule.local_host.clone(),
            rule.local_port,
        );
    }
    Ok(())
}

/// One locally bound listener: either a fixed `ssh -L` rule or an `ssh -D` SOCKS5 listener.
#[derive(Debug, Clone)]
enum BoundRule {
    Local(LocalForward),
    Dynamic(DynamicForward),
}

/// A bound forward: its rule, and every socket it listens on.
type Bound = (BoundRule, Vec<TcpListener>);

/// Binds every local and dynamic forward, or none: a tunnel missing one of its
/// ports would look up while quietly not serving it.
async fn bind_all(
    local_forwards: &[LocalForward],
    dynamic_forwards: &[DynamicForward],
) -> Result<Vec<Bound>, String> {
    let mut bound = Vec::with_capacity(local_forwards.len() + dynamic_forwards.len());
    for forward in local_forwards {
        bound.push((
            BoundRule::Local(forward.clone()),
            bind_address(forward.bind_address.as_deref(), forward.bind_port).await?,
        ));
    }
    for forward in dynamic_forwards {
        bound.push((
            BoundRule::Dynamic(forward.clone()),
            bind_address(forward.bind_address.as_deref(), forward.bind_port).await?,
        ));
    }
    Ok(bound)
}

/// Binds one forward's addresses. The loopback is IPv4 plus, where the system
/// has one, IPv6 — as ssh does — so `localhost` reaches the tunnel whichever
/// family a client tries first.
async fn bind_address(bind_addr: Option<&str>, port: u16) -> Result<Vec<TcpListener>, String> {
    let failed = |address: &str, e: io::Error| format!("cannot listen on {address}:{port}: {e}");
    let (required, optional): (Vec<SocketAddr>, Vec<SocketAddr>) = match bind_addr {
        None | Some("localhost") => (
            vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))],
            vec![SocketAddr::from((Ipv6Addr::LOCALHOST, port))],
        ),
        Some("" | "*") => (
            vec![SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))],
            vec![],
        ),
        Some(address) => match address.parse::<IpAddr>() {
            Ok(ip) => (vec![SocketAddr::new(ip, port)], vec![]),
            Err(_) => (
                tokio::net::lookup_host((address, port))
                    .await
                    .map_err(|e| failed(address, e))?
                    .collect(),
                vec![],
            ),
        },
    };

    let mut listeners = Vec::new();
    for address in required {
        listeners.push(
            TcpListener::bind(address)
                .await
                .map_err(|e| failed(&address.ip().to_string(), e))?,
        );
    }
    for address in optional {
        match TcpListener::bind(address).await {
            Ok(listener) => listeners.push(listener),
            Err(e) => tracing::debug!(%address, error = %e, "optional forward address skipped"),
        }
    }
    Ok(listeners)
}

/// Serves the forwards over `conn` until the connection closes, and returns why.
///
/// Every accepted connection runs as its own task; they all end with this call,
/// since their channels die with the connection anyway.
async fn forward(
    conn: Arc<SshConnection>,
    listeners: &[Bound],
    tx: &mpsc::Sender<CoreEvent>,
    name: &str,
) -> String {
    let mut streams = JoinSet::new();
    let mut liveness = time::interval(LIVENESS_CHECK);
    loop {
        tokio::select! {
            (index, accepted) = accept(listeners), if !listeners.is_empty() => match accepted {
                Ok((socket, peer)) => {
                    let rule = listeners[index].0.clone();
                    let (tx, name) = (tx.clone(), name.to_string());
                    match rule {
                        BoundRule::Local(local) => {
                            streams.spawn(carry(Arc::clone(&conn), local, socket, peer, tx, name));
                        }
                        BoundRule::Dynamic(dynamic) => {
                            streams.spawn(carry_socks5(
                                Arc::clone(&conn),
                                dynamic,
                                socket,
                                peer,
                                tx,
                                name,
                            ));
                        }
                    }
                }
                // Per-connection trouble such as a full file table; the listener
                // itself is fine, so back off briefly instead of spinning.
                Err(e) => {
                    tracing::warn!(error = %e, "accepting a forwarded connection failed");
                    time::sleep(Duration::from_millis(100)).await;
                }
            },
            _ = liveness.tick() => {
                if conn.is_closed() {
                    return String::from("connection lost");
                }
            }
            Some(_) = streams.join_next() => {}
        }
    }
}

/// The next connection on any forward's listeners, with the forward's index.
async fn accept(listeners: &[Bound]) -> (usize, io::Result<(TcpStream, SocketAddr)>) {
    poll_fn(|cx| {
        for (index, (_, sockets)) in listeners.iter().enumerate() {
            for socket in sockets {
                if let Poll::Ready(accepted) = socket.poll_accept(cx) {
                    return Poll::Ready((index, accepted));
                }
            }
        }
        Poll::Pending
    })
    .await
}

/// Carries one accepted connection over its own `direct-tcpip` channel.
///
/// The open waits as long as the server takes to reach the target, as ssh does:
/// giving up on it early would leave a channel the server still opens.
async fn carry(
    conn: Arc<SshConnection>,
    forward: LocalForward,
    socket: TcpStream,
    peer: SocketAddr,
    tx: mpsc::Sender<CoreEvent>,
    name: String,
) {
    let opened = conn
        .channel_open_direct_tcpip(
            forward.remote_host.as_str(),
            u32::from(forward.remote_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await;
    let mut channel = match opened {
        Ok(channel) => channel,
        // The tunnel itself is up, so its status stays; say why this one
        // connection went nowhere — a stopped service, a server that forbids
        // forwarding — or the user only sees a reset.
        Err(e) => {
            let message = format!("Tunnel to '{name}': {forward} could not be opened: {e}");
            let _ = tx.send(CoreEvent::Error(message)).await;
            return;
        }
    };
    if let Err(e) = pump(socket, &mut channel).await {
        tracing::debug!(%forward, error = %e, "forwarded connection ended");
    }
    // A channel does not close when dropped: without this, every connection that
    // ended abruptly would hold its channel, and the server's socket to the
    // target, until the tunnel itself went down.
    let _ = channel.close().await;
}

/// Handles one incoming client connection on a `ssh -D` local SOCKS5 listener (RFC 1928).
async fn carry_socks5(
    conn: Arc<SshConnection>,
    forward: DynamicForward,
    mut socket: TcpStream,
    peer: SocketAddr,
    tx: mpsc::Sender<CoreEvent>,
    name: String,
) {
    // 1. Greeting: VER (0x05), NMETHODS, METHODS...
    let mut head = [0u8; 2];
    if socket.read_exact(&mut head).await.is_err() || head[0] != 0x05 {
        return;
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    if socket.read_exact(&mut methods).await.is_err() {
        return;
    }
    if !methods.contains(&0x00) {
        let _ = socket.write_all(&[0x05, 0xFF]).await;
        return;
    }
    if socket.write_all(&[0x05, 0x00]).await.is_err() {
        return;
    }

    // 2. Request: VER (0x05), CMD (0x01 = CONNECT), RSV, ATYP, DST.ADDR, DST.PORT.
    let mut req = [0u8; 4];
    if socket.read_exact(&mut req).await.is_err() || req[0] != 0x05 {
        return;
    }
    if req[1] != 0x01 {
        // Command not supported.
        let _ = socket
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await;
        return;
    }

    let dest_host = match req[3] {
        0x01 => {
            let mut ip = [0u8; 4];
            if socket.read_exact(&mut ip).await.is_err() {
                return;
            }
            Ipv4Addr::from(ip).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            if socket.read_exact(&mut len).await.is_err() {
                return;
            }
            let mut dom = vec![0u8; usize::from(len[0])];
            if socket.read_exact(&mut dom).await.is_err() {
                return;
            }
            match String::from_utf8(dom) {
                Ok(s) => s,
                Err(_) => return,
            }
        }
        0x04 => {
            let mut ip = [0u8; 16];
            if socket.read_exact(&mut ip).await.is_err() {
                return;
            }
            Ipv6Addr::from(ip).to_string()
        }
        _ => {
            let _ = socket
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return;
        }
    };

    let mut port_bytes = [0u8; 2];
    if socket.read_exact(&mut port_bytes).await.is_err() {
        return;
    }
    let dest_port = u16::from_be_bytes(port_bytes);

    let opened = conn
        .channel_open_direct_tcpip(
            dest_host.as_str(),
            u32::from(dest_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        )
        .await;
    let mut channel = match opened {
        Ok(ch) => ch,
        Err(e) => {
            let _ = socket
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            let message = format!(
                "SOCKS5 tunnel to '{name}' ({forward}): {dest_host}:{dest_port} failed: {e}"
            );
            let _ = tx.send(CoreEvent::Error(message)).await;
            return;
        }
    };

    if socket
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
        .is_err()
    {
        let _ = channel.close().await;
        return;
    }

    if let Err(e) = pump(socket, &mut channel).await {
        tracing::debug!(%forward, %dest_host, dest_port, error = %e, "SOCKS5 connection ended");
    }
    let _ = channel.close().await;
}

/// Copies both ways, passing EOF along, until the remote side is done.
///
/// A local EOF only half-closes: the reply still comes back. The remote end
/// finishes it all, as ssh closes the local socket once the channel closes — the
/// reader cannot tell a remote EOF from a close, and writing on after a close
/// spins in russh until the local peer gives up.
async fn pump(
    socket: TcpStream,
    channel: &mut russh::Channel<russh::client::Msg>,
) -> io::Result<()> {
    let (mut local_rx, mut local_tx) = socket.into_split();
    let mut remote_tx = channel.make_writer();
    let mut remote_rx = channel.make_reader();
    let upstream = async {
        tokio::io::copy(&mut local_rx, &mut remote_tx).await?;
        remote_tx.shutdown().await
    };
    let downstream = async {
        tokio::io::copy(&mut remote_rx, &mut local_tx).await?;
        local_tx.shutdown().await
    };
    tokio::pin!(upstream, downstream);
    let mut sending = true;
    loop {
        tokio::select! {
            done = &mut downstream => return done,
            done = &mut upstream, if sending => {
                done?;
                sending = false;
            }
        }
    }
}

async fn send_status(tx: &mpsc::Sender<CoreEvent>, name: &str, status: TunnelStatus) {
    let _ = tx
        .send(CoreEvent::TunnelStatusChanged(name.to_string(), status))
        .await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(bind: Option<&str>, port: u16, host: &str, hostport: u16) -> LocalForward {
        LocalForward {
            bind_address: bind.map(str::to_string),
            bind_port: port,
            remote_host: host.to_string(),
            remote_port: hostport,
        }
    }

    #[test]
    fn the_ssh_notation_parses() {
        let cases = [
            ("9443:127.0.0.1:9443", rule(None, 9443, "127.0.0.1", 9443)),
            (
                "5432:db.internal:5432",
                rule(None, 5432, "db.internal", 5432),
            ),
            (
                "localhost:8080:web:80",
                rule(Some("localhost"), 8080, "web", 80),
            ),
            (
                "0.0.0.0:8080:web:80",
                rule(Some("0.0.0.0"), 8080, "web", 80),
            ),
            ("*:8080:web:80", rule(Some("*"), 8080, "web", 80)),
            (":8080:web:80", rule(Some(""), 8080, "web", 80)),
            (
                "[::1]:8080:[fe80::1]:80",
                rule(Some("::1"), 8080, "fe80::1", 80),
            ),
            ("8080:[::1]:80", rule(None, 8080, "::1", 80)),
            (
                "  3000:localhost:3000  ",
                rule(None, 3000, "localhost", 3000),
            ),
        ];
        for (spec, expected) in cases {
            assert_eq!(spec.parse::<LocalForward>(), Ok(expected), "{spec}");
        }
    }

    #[test]
    fn a_malformed_rule_is_rejected() {
        for spec in [
            "",
            "8080",
            "8080:web",
            "a:b:c:d:e",
            "0:web:80",
            "8080:web:0",
            "70000:web:80",
            "8080:web:http",
            "8080::80",
            "[::1:8080:web:80",
            "-1:web:80",
        ] {
            assert!(
                spec.parse::<LocalForward>().is_err(),
                "'{spec}' should be rejected"
            );
        }
    }

    #[test]
    fn a_rule_round_trips_through_its_notation() {
        for spec in [
            "9443:127.0.0.1:9443",
            "localhost:8080:web:80",
            ":8080:web:80",
            "[::1]:8080:[fe80::1]:80",
        ] {
            let parsed: LocalForward = spec.parse().expect("valid");
            assert_eq!(parsed.to_string(), spec);
        }
    }

    #[test]
    fn a_host_stores_its_forwards_in_the_ssh_notation() {
        let host = Host {
            name: String::from("nas"),
            hostname: String::from("10.0.0.5"),
            local_forwards: vec![rule(None, 9443, "127.0.0.1", 9443)],
            tunnel_autostart: true,
            ..Host::default()
        };
        let written = toml::to_string(&host).expect("serialize");
        assert!(
            written.contains("local_forwards = [\"9443:127.0.0.1:9443\"]"),
            "{written}"
        );
        assert!(written.contains("tunnel_autostart = true"), "{written}");

        let read: Host = toml::from_str(&written).expect("deserialize");
        assert_eq!(read.local_forwards, host.local_forwards);
        assert!(read.tunnel_autostart);
    }

    #[test]
    fn an_unforwarded_host_leaves_hosts_toml_untouched() {
        let host: Host =
            toml::from_str("name = \"web\"\nhostname = \"10.0.0.1\"\n").expect("parse");
        assert!(host.local_forwards.is_empty());
        assert!(!host.tunnel_autostart);

        let written = toml::to_string(&host).expect("serialize");
        assert!(!written.contains("local_forwards"), "{written}");
        assert!(!written.contains("tunnel_autostart"), "{written}");
    }

    #[test]
    fn only_a_tunnel_input_counts_as_a_change() {
        let before = Host {
            name: String::from("nas"),
            hostname: String::from("10.0.0.5"),
            local_forwards: vec![rule(None, 9443, "127.0.0.1", 9443)],
            ..Host::default()
        };

        let mut cosmetic = before.clone();
        cosmetic.tags = vec![String::from("home")];
        cosmetic.notes = Some(String::from("portainer"));
        cosmetic.tunnel_autostart = true;
        assert!(!changed(&before, &cosmetic));

        let mut moved = before.clone();
        moved
            .local_forwards
            .push(rule(None, 5432, "localhost", 5432));
        assert!(changed(&before, &moved));

        let mut readdressed = before.clone();
        readdressed.hostname = String::from("10.0.0.6");
        assert!(changed(&before, &readdressed));

        let mut narrowed = before.clone();
        narrowed.identities_only = true;
        assert!(changed(&before, &narrowed));
    }

    #[tokio::test]
    async fn a_port_in_use_fails_the_whole_tunnel_and_binds_nothing() {
        let taken = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let busy = taken.local_addr().expect("addr").port();
        let free = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let spare = free.local_addr().expect("addr").port();
        drop(free);

        let forwards = [
            rule(None, spare, "localhost", 80),
            rule(None, busy, "localhost", 80),
        ];
        let err = bind_all(&forwards, &[])
            .await
            .expect_err("the busy port must fail");
        assert!(err.contains(&busy.to_string()), "{err}");

        // The forward that did bind was released along with the failure.
        TcpListener::bind(("127.0.0.1", spare))
            .await
            .expect("the spare port is free again");
    }

    #[test]
    fn remote_and_dynamic_forwards_parse_and_round_trip() {
        let rf: RemoteForward = "8080:127.0.0.1:3000".parse().expect("valid -R");
        assert_eq!(rf.bind_address, None);
        assert_eq!(rf.remote_port, 8080);
        assert_eq!(rf.local_host, "127.0.0.1");
        assert_eq!(rf.local_port, 3000);
        assert_eq!(rf.to_string(), "8080:127.0.0.1:3000");

        let rf_bind: RemoteForward = "0.0.0.0:9000:localhost:80".parse().expect("valid bind -R");
        assert_eq!(rf_bind.bind_address.as_deref(), Some("0.0.0.0"));
        assert_eq!(rf_bind.to_string(), "0.0.0.0:9000:localhost:80");
        assert!("invalid".parse::<RemoteForward>().is_err());

        let df: DynamicForward = "1080".parse().expect("valid -D");
        assert_eq!(df.bind_address, None);
        assert_eq!(df.bind_port, 1080);
        assert_eq!(df.to_string(), "1080");

        let df_bind: DynamicForward = "127.0.0.1:10808".parse().expect("valid bind -D");
        assert_eq!(df_bind.bind_address.as_deref(), Some("127.0.0.1"));
        assert_eq!(df_bind.to_string(), "127.0.0.1:10808");
        assert!("0".parse::<DynamicForward>().is_err());
    }
}
