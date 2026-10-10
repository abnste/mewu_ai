// SPDX-License-Identifier: MPL-2.0
//! One policy for new outbound HTTP clients. Local Agent/MCP endpoints stay direct.
use crate::system_preferences::{validate_proxy, ProxyMode, SystemPreferences};
use std::sync::{OnceLock, RwLock};

#[derive(Clone)]
struct Policy {
    mode: ProxyMode,
    url: String,
}
static POLICY: OnceLock<RwLock<Result<Policy, String>>> = OnceLock::new();
fn policy() -> &'static RwLock<Result<Policy, String>> {
    POLICY.get_or_init(|| {
        RwLock::new(Ok(Policy {
            mode: ProxyMode::System,
            url: String::new(),
        }))
    })
}
pub fn initialize(value: Result<SystemPreferences, crate::system_preferences::Error>) {
    let next = value
        .map(|value| Policy {
            mode: value.network_proxy_mode,
            url: value.network_proxy_url,
        })
        .map_err(|e| e.to_string());
    if let Ok(mut state) = policy().write() {
        *state = next;
    }
}
pub fn configure(value: &SystemPreferences) -> Result<(), String> {
    validate_proxy(value.network_proxy_mode, &value.network_proxy_url)
        .map_err(|e| e.to_string())?;
    *policy().write().map_err(|_| "网络代理设置不可用")? = Ok(Policy {
        mode: value.network_proxy_mode,
        url: value.network_proxy_url.clone(),
    });
    Ok(())
}
pub fn is_local(url: &url::Url) -> bool {
    matches!(url.host(), Some(url::Host::Domain("localhost")))
        || matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
}
pub fn apply(
    builder: reqwest::ClientBuilder,
    target: Option<&url::Url>,
) -> Result<reqwest::ClientBuilder, String> {
    if target.is_some_and(is_local) {
        return Ok(builder.no_proxy());
    }
    let value = policy().read().map_err(|_| "网络代理设置不可用")?.clone()?;
    apply_policy(builder, &value)
}
fn apply_policy(
    builder: reqwest::ClientBuilder,
    value: &Policy,
) -> Result<reqwest::ClientBuilder, String> {
    match value.mode {
        ProxyMode::System => Ok(builder),
        ProxyMode::Direct => Ok(builder.no_proxy()),
        ProxyMode::Custom => Ok(builder
            .no_proxy()
            .proxy(reqwest::Proxy::all(&value.url).map_err(|_| "自定义代理地址无效")?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_endpoints_never_go_to_a_remote_proxy() {
        for target in [
            "http://localhost:7788",
            "http://127.0.0.1:7788",
            "http://[::1]:7788",
        ] {
            assert!(is_local(&url::Url::parse(target).unwrap()));
        }
        for target in [
            "https://api.deepseek.com",
            "http://localhost.evil",
            "http://192.168.1.10",
        ] {
            assert!(!is_local(&url::Url::parse(target).unwrap()));
        }
    }
    #[tokio::test]
    async fn custom_proxy_routes_an_actual_http_request_without_upstream_dns() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let observer = std::thread::spawn(move || {
            let request = server
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .expect("proxy must receive request");
            assert_eq!(request.url(), "http://mewu-proxy-test.invalid/models");
            request
                .respond(tiny_http::Response::from_string("proxy receipt"))
                .unwrap();
        });
        let value = Policy {
            mode: ProxyMode::Custom,
            url: format!("http://{address}"),
        };
        let client = apply_policy(
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)),
            &value,
        )
        .unwrap()
        .build()
        .unwrap();
        let response = client
            .get("http://mewu-proxy-test.invalid/models")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "proxy receipt");
        observer.join().unwrap();
    }
    #[tokio::test]
    async fn socks5_proxy_negotiates_and_routes_http_without_a_real_upstream() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let observer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut greeting = [0; 2];
            stream.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0; greeting[1] as usize];
            stream.read_exact(&mut methods).unwrap();
            assert!(methods.contains(&0));
            stream.write_all(&[5, 0]).unwrap();
            let mut connect = [0; 10];
            stream.read_exact(&mut connect).unwrap();
            assert_eq!(connect, [5, 1, 0, 1, 198, 51, 100, 7, 0, 80]);
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() <= 4096);
            }
            assert!(request.starts_with(b"GET /models HTTP/1.1\r\n"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nSOCKS receipt").unwrap();
        });
        let client = apply_policy(
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)),
            &Policy {
                mode: ProxyMode::Custom,
                url: format!("socks5://{address}"),
            },
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            client
                .get("http://198.51.100.7/models")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "SOCKS receipt"
        );
        observer.join().unwrap();
    }
}
