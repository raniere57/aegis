//! Lightweight DNS53 proxy: allow → block → cache → upstream.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::serialize::binary::{BinDecodable, BinEncodable};
use tokio::net::UdpSocket;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::cache::{CacheKey, DnsCache};
use crate::config::Config;
use crate::metrics::Metrics;
use crate::normalize::{is_local_bypass, normalize_domain};
use crate::recent::RecentBlocks;
use crate::trie::{Allowlist, Blocklist};

pub struct FilterState {
    pub blocklist: ArcSwap<Blocklist>,
    pub allowlist: ArcSwap<Allowlist>,
    pub domain_count: AtomicUsize,
    pub list_updated_at_unix: AtomicUsize,
    pub last_update_error: ArcSwap<Option<String>>,
    /// Rolling window of what was blocked, for the "why is this site broken?" UI.
    pub recent: RecentBlocks,
}

impl FilterState {
    pub fn new(blocklist: Blocklist, allowlist: Allowlist) -> Self {
        let count = blocklist.len();
        Self {
            blocklist: ArcSwap::from_pointee(blocklist),
            allowlist: ArcSwap::from_pointee(allowlist),
            domain_count: AtomicUsize::new(count),
            list_updated_at_unix: AtomicUsize::new(0),
            last_update_error: ArcSwap::from_pointee(None),
            recent: RecentBlocks::new(),
        }
    }

    pub fn swap_blocklist(&self, bl: Blocklist) {
        let count = bl.len();
        self.domain_count.store(count, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(0);
        self.list_updated_at_unix.store(now, Ordering::Relaxed);
        self.blocklist.store(Arc::new(bl));
    }
}

pub struct DnsProxy {
    pub config: Arc<ArcSwap<Config>>,
    pub metrics: Arc<Metrics>,
    pub filter: Arc<FilterState>,
    pub cache: Arc<DnsCache>,
    inflight: Arc<Semaphore>,
    /// Index of the upstream that answered last. Without this, a silently dropped primary
    /// costs a full timeout on every single cache miss, forever.
    preferred_upstream: AtomicUsize,
}

impl DnsProxy {
    pub fn new(
        config: Arc<ArcSwap<Config>>,
        metrics: Arc<Metrics>,
        filter: Arc<FilterState>,
    ) -> Self {
        let cfg = config.load();
        let cache = Arc::new(DnsCache::new(cfg.cache.size, cfg.cache.nxdomain_ttl_secs));
        let inflight = Arc::new(Semaphore::new(cfg.upstream.max_inflight.max(1)));
        Self {
            config,
            metrics,
            filter,
            cache,
            inflight,
            preferred_upstream: AtomicUsize::new(0),
        }
    }

    pub async fn run(self: Arc<Self>, addrs: Vec<SocketAddr>) -> anyhow::Result<()> {
        let mut handles = Vec::new();
        for addr in addrs {
            let sock = Arc::new(UdpSocket::bind(addr).await?);
            tracing::info!(%addr, "DNS UDP listening");
            let this = Arc::clone(&self);
            let sock_loop = Arc::clone(&sock);
            handles.push(tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                loop {
                    match sock_loop.recv_from(&mut buf).await {
                        Ok((n, peer)) => {
                            let req = buf[..n].to_vec();
                            let this2 = Arc::clone(&this);
                            let sock2 = Arc::clone(&sock_loop);
                            tokio::spawn(async move {
                                if let Some(resp) = this2.handle_query(&req).await {
                                    let _ = sock2.send_to(&resp, peer).await;
                                }
                            });
                        }
                        Err(e) => {
                            warn!(error = %e, "udp recv error");
                        }
                    }
                }
            }));

            // TCP (RFC 7766) — length-prefixed messages
            let this_tcp = Arc::clone(&self);
            handles.push(tokio::spawn(async move {
                let listener = match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => {
                        tracing::info!(%addr, "DNS TCP listening");
                        l
                    }
                    Err(e) => {
                        warn!(%addr, error = %e, "TCP bind failed");
                        return;
                    }
                };
                loop {
                    // Never `continue` straight into accept() on error: under EMFILE this
                    // busy-spins a core at 100%, violating the idle-CPU budget and starving
                    // the UDP path. Back off instead.
                    let (mut stream, _) = match listener.accept().await {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(%addr, error = %e, "TCP accept failed");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                    };
                    let this2 = Arc::clone(&this_tcp);
                    tokio::spawn(async move {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        let mut lenbuf = [0u8; 2];
                        if stream.read_exact(&mut lenbuf).await.is_err() {
                            return;
                        }
                        let len = u16::from_be_bytes(lenbuf) as usize;
                        if len == 0 || len > 65535 {
                            return;
                        }
                        let mut req = vec![0u8; len];
                        if stream.read_exact(&mut req).await.is_err() {
                            return;
                        }
                        if let Some(resp) = this2.handle_query(&req).await {
                            let rlen = (resp.len() as u16).to_be_bytes();
                            let _ = stream.write_all(&rlen).await;
                            let _ = stream.write_all(&resp).await;
                        }
                    });
                }
            }));
        }
        for h in handles {
            let _ = h.await;
        }
        Ok(())
    }

    async fn handle_query(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        self.metrics.queries.fetch_add(1, Ordering::Relaxed);

        let request = match Message::from_bytes(bytes) {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "bad dns message");
                return None;
            }
        };

        if request.message_type() != MessageType::Query || request.op_code() != OpCode::Query {
            return None;
        }

        // Exactly one question. The whole message is forwarded upstream and the reply is
        // cached under question 1's key, so a 2-question query lets any local process poison
        // one (name, type) with a FORMERR for the default TTL.
        if request.queries().len() != 1 {
            return Some(formerr(&request));
        }
        let question = &request.queries()[0];

        // to_ascii(), not to_string(): hickory's Display decodes punycode back to Unicode, so
        // an IDN normalizes to None and `unwrap_or_default()` collapses it — along with
        // localhost, over-long names and the root query — onto the shared cache key "".
        let mut qname = question.name().to_ascii();
        if qname.ends_with('.') {
            qname.pop();
        }
        let domain = normalize_domain(&qname).unwrap_or_default();
        let qtype = question.query_type();
        let qclass = question.query_class();

        let filtering = self.metrics.filtering.load(Ordering::Relaxed)
            && self.config.load().daemon.enabled;

        let mut allowlisted = false;
        if filtering && !domain.is_empty() && !is_local_bypass(&domain) {
            allowlisted = self.filter.allowlist.load().contains_normalized(&domain);
            if !allowlisted {
                let block = self.filter.blocklist.load();
                if !block.is_empty() && block.contains_normalized(&domain) {
                    self.metrics.blocked.fetch_add(1, Ordering::Relaxed);
                    self.filter.recent.record(&domain);
                    return Some(nxdomain_reflect(bytes));
                }
            }
        }

        // qname (ASCII, never empty) is the key; `domain` is only the filtering view of it.
        let key = CacheKey {
            name: qname,
            qtype: u16::from(qtype),
            qclass: u16::from(qclass),
            dnssec_ok: request
                .extensions()
                .as_ref()
                .is_some_and(|e| e.flags().dnssec_ok),
        };

        if let Some(mut cached) = self.cache.get(&key) {
            self.metrics.cache_hit.fetch_add(1, Ordering::Relaxed);
            set_message_id(&mut cached, request.id());
            return Some(cached);
        }
        self.metrics.cache_miss.fetch_add(1, Ordering::Relaxed);

        let _permit = match self.inflight.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                self.metrics
                    .upstream_errors
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .consecutive_upstream_failures
                    .fetch_add(1, Ordering::Relaxed);
                return Some(servfail(&request));
            }
        };

        match self.forward_upstream(bytes).await {
            Some(resp_bytes) => {
                self.metrics.upstream_ok.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .consecutive_upstream_failures
                    .store(0, Ordering::Relaxed);
                if let Ok(resp_msg) = Message::from_bytes(&resp_bytes) {
                    // Trackers hide behind first-party CNAMEs, so QNAME filtering alone misses
                    // them entirely. The response is already parsed here for min_ttl.
                    if filtering && !allowlisted && self.cname_blocked(&resp_msg) {
                        self.metrics.blocked.fetch_add(1, Ordering::Relaxed);
                        self.filter.recent.record(&domain);
                        let blocked = nxdomain_reflect(bytes);
                        let mut store = blocked.clone();
                        set_message_id(&mut store, 0);
                        self.cache.insert(key, store, self.cache.nxdomain_ttl(), true);
                        return Some(blocked);
                    }
                    // A truncated answer must never be cached: every later client would get
                    // the same truncation, and its TCP retry lands back on this same cache.
                    if !resp_msg.truncated() {
                        let ttl = min_ttl(&resp_msg).unwrap_or(60);
                        let is_nx = resp_msg.response_code() == ResponseCode::NXDomain;
                        let mut store = resp_bytes.clone();
                        set_message_id(&mut store, 0);
                        self.cache.insert(key, store, ttl, is_nx);
                    }
                }
                let mut out = resp_bytes;
                set_message_id(&mut out, request.id());
                Some(out)
            }
            None => {
                self.metrics
                    .upstream_errors
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .consecutive_upstream_failures
                    .fetch_add(1, Ordering::Relaxed);
                Some(servfail(&request))
            }
        }
    }

    /// True when any CNAME target in the answer chain is blocked. Runs on the cache-miss path
    /// only, which has already spent 5-30 ms on the network — measured 0.32 µs per record.
    fn cname_blocked(&self, resp: &Message) -> bool {
        let block = self.filter.blocklist.load();
        if block.is_empty() {
            return false;
        }
        let allow = self.filter.allowlist.load();
        resp.answers().iter().any(|rr| {
            let Some(target) = rr.data().as_cname() else {
                return false;
            };
            let mut name = target.0.to_ascii();
            if name.ends_with('.') {
                name.pop();
            }
            let Some(name) = normalize_domain(&name) else {
                return false;
            };
            !allow.contains_normalized(&name) && block.contains_normalized(&name)
        })
    }

    async fn forward_upstream(&self, query: &[u8]) -> Option<Vec<u8>> {
        let cfg = self.config.load();
        let timeout = Duration::from_millis(cfg.upstream.timeout_ms.max(100));

        // Parse once per call rather than once per attempt, and drop any upstream that points
        // back at one of our own listeners — that would forward to ourselves, miss, take
        // another permit, and forward again until every inflight permit is pinned at 100% CPU.
        let listen: Vec<SocketAddr> = cfg
            .daemon
            .listen
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        let servers: Vec<(&String, SocketAddr)> = cfg
            .upstream
            .servers
            .iter()
            .filter_map(|s| s.parse().ok().map(|a| (s, a)))
            .filter(|(s, a)| {
                if listen.contains(a) {
                    warn!(server = %s, "ignoring upstream that points at our own listener");
                    false
                } else {
                    true
                }
            })
            .collect();
        if servers.is_empty() {
            return None;
        }

        let start = self.preferred_upstream.load(Ordering::Relaxed) % servers.len();
        for offset in 0..servers.len() {
            let idx = (start + offset) % servers.len();
            let (server, addr) = servers[idx];
            match tokio::time::timeout(timeout, exchange_udp(addr, query)).await {
                Ok(Ok(resp)) => {
                    if offset != 0 {
                        self.preferred_upstream.store(idx, Ordering::Relaxed);
                    }
                    return Some(resp);
                }
                Ok(Err(e)) => {
                    debug!(server = %server, error = %e, "upstream error");
                }
                Err(_) => {
                    debug!(server = %server, "upstream timeout");
                }
            }
        }
        None
    }
}

async fn exchange_udp(addr: SocketAddr, query: &[u8]) -> std::io::Result<Vec<u8>> {
    let bind = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let sock = UdpSocket::bind(bind).await?;
    sock.connect(addr).await?;
    sock.send(query).await?;
    let mut buf = vec![0u8; 4096];
    let n = sock.recv(&mut buf).await?;
    buf.truncate(n);
    Ok(buf)
}

fn set_message_id(msg: &mut [u8], id: u16) {
    if msg.len() >= 2 {
        msg[0] = (id >> 8) as u8;
        msg[1] = id as u8;
    }
}

/// A blocked NXDOMAIN is the request with four header bits changed, so reflect the bytes
/// instead of re-encoding. Building it through `Message` spins up a BinEncoder with a name
/// compression table for a message that has no names to compress — 1.456 µs of the measured
/// 4.177 µs block path, and ~19 allocations. Reflecting also preserves the client's EDNS OPT,
/// which the from-scratch builder silently dropped.
fn nxdomain_reflect(request: &[u8]) -> Vec<u8> {
    let mut out = request.to_vec();
    if out.len() < 12 {
        return out;
    }
    out[2] |= 0x80; // QR = response
    out[2] &= !0x02; // TC = 0
    // Literal, not `(out[3] & 0xF0) | 0x03`: masking would *preserve* the request's AD and CD
    // bits into our synthetic answer.
    out[3] = 0x80 | 0x03; // RA = 1, Z/AD/CD = 0, RCODE = NXDOMAIN
    out
}

/// Malformed request shape (not exactly one question). Reflect and set RCODE = FORMERR.
fn formerr(request: &Message) -> Vec<u8> {
    let mut msg = Message::new();
    msg.set_id(request.id());
    msg.set_message_type(MessageType::Response);
    msg.set_op_code(OpCode::Query);
    msg.set_response_code(ResponseCode::FormErr);
    msg.set_recursion_available(true);
    msg.set_recursion_desired(request.recursion_desired());
    msg.to_bytes().unwrap_or_default()
}

fn servfail(request: &Message) -> Vec<u8> {
    let mut msg = Message::new();
    msg.set_id(request.id());
    msg.set_message_type(MessageType::Response);
    msg.set_op_code(OpCode::Query);
    msg.set_response_code(ResponseCode::ServFail);
    msg.set_recursion_available(true);
    msg.set_recursion_desired(request.recursion_desired());
    for q in request.queries() {
        msg.add_query(q.clone());
    }
    msg.to_bytes().unwrap_or_default()
}

fn min_ttl(msg: &Message) -> Option<u32> {
    let mut min = None;
    for rr in msg
        .answers()
        .iter()
        .chain(msg.name_servers().iter())
        .chain(msg.additionals().iter())
    {
        let ttl = rr.ttl();
        min = Some(min.map_or(ttl, |m: u32| m.min(ttl)));
    }
    min
}

