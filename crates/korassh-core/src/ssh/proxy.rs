//! Upstream HTTP CONNECT and SOCKS5 (RFC 1928 / RFC 1929) proxy dialling for SSH.
//!
//! Pure Tokio TCP implementation — zero native C dependencies, works identically
//! across Linux (gnu + musl), macOS, and Windows.

use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Protocol of an upstream proxy used to reach an SSH host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Http,
    Socks5,
}

/// Optional upstream proxy configuration stored on a [`crate::ssh::client::Host`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Opens a TCP connection to `target_host:target_port` tunnelled through `proxy`.
pub async fn connect_via_proxy(
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream> {
    if proxy.host.trim().is_empty() || proxy.port == 0 {
        bail!("invalid proxy address '{}:{}'", proxy.host, proxy.port);
    }
    let addr = format!("{}:{}", proxy.host.trim(), proxy.port);
    let mut stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("connect to proxy {addr}"))?;

    match proxy.kind {
        ProxyKind::Http => {
            http_connect_handshake(&mut stream, proxy, target_host, target_port).await?
        }
        ProxyKind::Socks5 => socks5_handshake(&mut stream, proxy, target_host, target_port).await?,
    }

    Ok(stream)
}

async fn http_connect_handshake(
    stream: &mut TcpStream,
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<()> {
    let authority = if target_host.contains(':') && !target_host.starts_with('[') {
        format!("[{target_host}]:{target_port}")
    } else {
        format!("{target_host}:{target_port}")
    };

    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(user) = proxy.username.as_deref().filter(|u| !u.is_empty()) {
        let pass = proxy
            .password
            .as_deref()
            .filter(|p| !crate::config::vault::is_encrypted(p))
            .unwrap_or("");
        let token = B64.encode(format!("{user}:{pass}"));
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("\r\n");

    stream
        .write_all(req.as_bytes())
        .await
        .context("send HTTP CONNECT request")?;

    // Read response header byte-by-byte up to \r\n\r\n so we never over-read the
    // SSH server's banner bytes that immediately follow the proxy header.
    let mut header = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    const MAX_HEADER: usize = 8192;
    while header.len() < MAX_HEADER {
        let n = stream
            .read(&mut byte)
            .await
            .context("read HTTP CONNECT response")?;
        if n == 0 {
            bail!("HTTP proxy closed the connection during CONNECT");
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if !header.ends_with(b"\r\n\r\n") {
        bail!("HTTP proxy returned an oversized or malformed header");
    }
    let header_str = String::from_utf8_lossy(&header);
    let status_line = header_str.lines().next().unwrap_or("");
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok());
    if code != Some(200) {
        bail!("HTTP proxy rejected CONNECT: {status_line}");
    }
    Ok(())
}

async fn socks5_handshake(
    stream: &mut TcpStream,
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<()> {
    let user = proxy.username.as_deref().filter(|u| !u.is_empty());
    let pass = proxy
        .password
        .as_deref()
        .filter(|p| !crate::config::vault::is_encrypted(p))
        .unwrap_or("");

    // 1. Greeting: offer NO_AUTH (0x00) and optionally USER_PASS (0x02).
    let greeting: Vec<u8> = if user.is_some() {
        vec![0x05, 0x02, 0x00, 0x02]
    } else {
        vec![0x05, 0x01, 0x00]
    };
    stream
        .write_all(&greeting)
        .await
        .context("send SOCKS5 greeting")?;

    let mut choice = [0u8; 2];
    stream
        .read_exact(&mut choice)
        .await
        .context("read SOCKS5 auth method")?;
    if choice[0] != 0x05 {
        bail!("SOCKS5 proxy returned unsupported version {}", choice[0]);
    }
    match choice[1] {
        0x00 => {} // No authentication required.
        0x02 => {
            let Some(u) = user else {
                bail!("SOCKS5 proxy requires username/password authentication");
            };
            if u.len() > 255 || pass.len() > 255 {
                bail!("SOCKS5 proxy credentials exceed 255 bytes");
            }
            let mut auth = Vec::with_capacity(3 + u.len() + pass.len());
            auth.push(0x01); // subnegotiation version
            auth.push(u.len() as u8);
            auth.extend_from_slice(u.as_bytes());
            auth.push(pass.len() as u8);
            auth.extend_from_slice(pass.as_bytes());
            stream
                .write_all(&auth)
                .await
                .context("send SOCKS5 credentials")?;
            let mut resp = [0u8; 2];
            stream
                .read_exact(&mut resp)
                .await
                .context("read SOCKS5 auth status")?;
            if resp[1] != 0x00 {
                bail!("SOCKS5 proxy authentication failed (status {})", resp[1]);
            }
        }
        0xFF => bail!("SOCKS5 proxy rejected all offered authentication methods"),
        other => bail!("SOCKS5 proxy selected unsupported auth method {other:#x}"),
    }

    // 2. CONNECT request.
    let mut req = Vec::with_capacity(7 + target_host.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00]); // VER=5, CMD=CONNECT, RSV=0
    let clean_host = target_host.trim_matches(|c| c == '[' || c == ']');
    if let Ok(ip4) = clean_host.parse::<Ipv4Addr>() {
        req.push(0x01);
        req.extend_from_slice(&ip4.octets());
    } else if let Ok(ip6) = clean_host.parse::<Ipv6Addr>() {
        req.push(0x04);
        req.extend_from_slice(&ip6.octets());
    } else {
        if clean_host.len() > 255 {
            bail!("target hostname too long for SOCKS5");
        }
        req.push(0x03);
        req.push(clean_host.len() as u8);
        req.extend_from_slice(clean_host.as_bytes());
    }
    req.extend_from_slice(&target_port.to_be_bytes());
    stream
        .write_all(&req)
        .await
        .context("send SOCKS5 CONNECT request")?;

    // 3. CONNECT reply: VER, REP, RSV, ATYP, BND.ADDR, BND.PORT.
    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .context("read SOCKS5 reply header")?;
    if head[0] != 0x05 {
        bail!("SOCKS5 reply had invalid version {}", head[0]);
    }
    if head[1] != 0x00 {
        bail!(
            "SOCKS5 proxy CONNECT failed: {}",
            socks5_reply_reason(head[1])
        );
    }
    match head[3] {
        0x01 => {
            let mut skip = [0u8; 4 + 2];
            stream.read_exact(&mut skip).await?;
        }
        0x04 => {
            let mut skip = [0u8; 16 + 2];
            stream.read_exact(&mut skip).await?;
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut skip = vec![0u8; usize::from(len[0]) + 2];
            stream.read_exact(&mut skip).await?;
        }
        atyp => {
            return Err(anyhow!(
                "SOCKS5 reply contained unknown address type {atyp}"
            ))
        }
    }

    Ok(())
}

fn socks5_reply_reason(code: u8) -> &'static str {
    match code {
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn http_connect_proxy_sends_auth_and_preserves_banner_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 512];
            let n = sock.read(&mut buf).await.expect("read");
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("CONNECT example.com:22 HTTP/1.1\r\n"));
            assert!(req.contains("Proxy-Authorization: Basic "));
            sock.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nSSH-2.0-Test\r\n")
                .await
                .expect("write");
        });

        let proxy = ProxyConfig {
            kind: ProxyKind::Http,
            host: "127.0.0.1".into(),
            port,
            username: Some("alice".into()),
            password: Some("pw".into()),
        };
        let mut stream = connect_via_proxy(&proxy, "example.com", 22)
            .await
            .expect("connect");
        let mut banner = [0u8; 14];
        stream.read_exact(&mut banner).await.expect("read banner");
        assert_eq!(&banner, b"SSH-2.0-Test\r\n");
        server.await.expect("server task");
    }

    #[tokio::test]
    async fn socks5_proxy_authenticates_and_connects_domain_target() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut greet = [0u8; 4];
            sock.read_exact(&mut greet).await.expect("greet");
            assert_eq!(greet, [0x05, 0x02, 0x00, 0x02]);
            // Choose username/password auth.
            sock.write_all(&[0x05, 0x02]).await.expect("choose");

            let mut uhead = [0u8; 2];
            sock.read_exact(&mut uhead).await.expect("uhead");
            assert_eq!(uhead[0], 0x01);
            let mut uname = vec![0u8; usize::from(uhead[1])];
            sock.read_exact(&mut uname).await.expect("uname");
            assert_eq!(uname, b"bob");
            let mut plen = [0u8; 1];
            sock.read_exact(&mut plen).await.expect("plen");
            let mut pass = vec![0u8; usize::from(plen[0])];
            sock.read_exact(&mut pass).await.expect("pass");
            assert_eq!(pass, b"secret");
            sock.write_all(&[0x01, 0x00]).await.expect("auth ok");

            // CONNECT header + domain.
            let mut req_head = [0u8; 5];
            sock.read_exact(&mut req_head).await.expect("req head");
            assert_eq!(&req_head[..4], &[0x05, 0x01, 0x00, 0x03]);
            let mut dom = vec![0u8; usize::from(req_head[4])];
            sock.read_exact(&mut dom).await.expect("domain");
            assert_eq!(dom, b"db.internal");
            let mut p = [0u8; 2];
            sock.read_exact(&mut p).await.expect("port");
            assert_eq!(u16::from_be_bytes(p), 2222);

            sock.write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                .await
                .expect("reply");
            sock.write_all(b"OK!").await.expect("payload");
        });

        let proxy = ProxyConfig {
            kind: ProxyKind::Socks5,
            host: "127.0.0.1".into(),
            port,
            username: Some("bob".into()),
            password: Some("secret".into()),
        };
        let mut stream = connect_via_proxy(&proxy, "db.internal", 2222)
            .await
            .expect("socks5 connect");
        let mut msg = [0u8; 3];
        stream.read_exact(&mut msg).await.expect("read msg");
        assert_eq!(&msg, b"OK!");
        server.await.expect("server");
    }
}
