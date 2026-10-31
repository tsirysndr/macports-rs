//! Native TCP connection timing, matching MacPorts' time_connect primitive.
use anyhow::{Context, Result, anyhow, ensure};
use std::{
    ffi::CString,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant},
};

/// Resolve the service and use the first returned TCP address, as upstream does.
/// The measurement excludes name lookup and does not perform an HTTP request.
pub async fn time_connect(host: &str, service: &str) -> Result<f64> {
    let host = CString::new(host).context("invalid probe hostname")?;
    let service = CString::new(service).context("invalid probe service")?;
    let address = tokio::task::spawn_blocking(move || resolve(&host, &service))
        .await
        .context("mirror resolver task failed")??;
    connect(address, Duration::from_secs(3)).await
}
async fn connect(address: SocketAddr, timeout: Duration) -> Result<f64> {
    let start = Instant::now();
    let stream = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(address))
        .await
        .context("mirror connection timed out")?
        .context("mirror connection failed")?;
    let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
    drop(stream);
    Ok(milliseconds)
}
fn resolve(host: &CString, service: &CString) -> Result<SocketAddr> {
    // SAFETY: zero is a valid addrinfo hint initialization; both strings are
    // NUL terminated and remain alive until getaddrinfo returns.
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = libc::AF_UNSPEC;
    hints.ai_socktype = libc::SOCK_STREAM;
    let mut result = std::ptr::null_mut();
    let status = unsafe { libc::getaddrinfo(host.as_ptr(), service.as_ptr(), &hints, &mut result) };
    ensure!(
        status == 0,
        "cannot resolve mirror host/service (getaddrinfo {status})"
    );
    struct Addresses(*mut libc::addrinfo);
    impl Drop for Addresses {
        fn drop(&mut self) {
            // SAFETY: this owns the successful getaddrinfo result exactly once.
            unsafe { libc::freeaddrinfo(self.0) };
        }
    }
    ensure!(
        !result.is_null(),
        "mirror hostname resolved to no addresses"
    );
    let addresses = Addresses(result);
    // SAFETY: the successful non-null result is live until Addresses drops.
    let first = unsafe { &*addresses.0 };
    ensure!(!first.ai_addr.is_null(), "mirror address is missing");
    match first.ai_family {
        libc::AF_INET => {
            ensure!(
                first.ai_addrlen as usize >= std::mem::size_of::<libc::sockaddr_in>(),
                "invalid IPv4 address length"
            );
            // SAFETY: family and length identify a valid sockaddr_in.
            let address = unsafe { &*first.ai_addr.cast::<libc::sockaddr_in>() };
            Ok(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes())),
                u16::from_be(address.sin_port),
            ))
        }
        libc::AF_INET6 => {
            ensure!(
                first.ai_addrlen as usize >= std::mem::size_of::<libc::sockaddr_in6>(),
                "invalid IPv6 address length"
            );
            // SAFETY: family and length identify a valid sockaddr_in6.
            let address = unsafe { &*first.ai_addr.cast::<libc::sockaddr_in6>() };
            Ok(std::net::SocketAddrV6::new(
                Ipv6Addr::from(address.sin6_addr.s6_addr),
                u16::from_be(address.sin6_port),
                u32::from_be(address.sin6_flowinfo),
                address.sin6_scope_id,
            )
            .into())
        }
        family => Err(anyhow!("unsupported mirror address family {family}")),
    }
}
