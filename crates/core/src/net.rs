// Network helpers shared by the chat providers: address checks, bounded reads
// and HTTP clients.
//
// Every answer a server sends is read with a ceiling, so a misbehaving or
// hostile server (a local model address can point anywhere) cannot make the
// app hold an unbounded amount of memory.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::Url;

use crate::i18n::{t, tf};

/// Ceiling for a JSON answer: a chat reply or a model list.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Ceiling for an error body: only its message is shown.
pub const MAX_ERROR_BODY: usize = 64 * 1024;

/// True for an address that is this machine: `localhost`, 127.0.0.0/8, ::1,
/// and the unspecified addresses (0.0.0.0, ::), which connect to this machine.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        Err(_) => false,
    }
}

pub fn is_loopback_url(url: &Url) -> bool {
    url.host_str().is_some_and(is_loopback_host)
}

/// The address of a model server as the user typed or pasted it, cleaned up:
/// a missing scheme becomes `http://`, trailing slashes and the documentation
/// sub-paths people paste (`/api`, `/v1`) go, and the loopback names become
/// 127.0.0.1 — on Windows `localhost` may resolve to ::1 first while Ollama
/// only listens on IPv4. Only http and https, and no `user:password@`.
pub fn normalise_server_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(t("Enter the server address first."));
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| tf("Not a valid address: {address}", &[("address", raw)]))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(t("The address must start with http:// or https://."));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(t("Leave the user name and password out of the address."));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(tf("Not a valid address: {address}", &[("address", raw)]));
    }
    if is_loopback_url(&url) {
        let _ = url.set_ip_host(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/api", "/v1"] {
        if let Some(rest) = path.strip_suffix(suffix) {
            path = rest.trim_end_matches('/').to_string();
        }
    }
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// `base` + `tail`, with exactly one slash between them.
pub fn join(base: &Url, tail: &str) -> String {
    format!("{}/{}", base.as_str().trim_end_matches('/'), tail.trim_start_matches('/'))
}

/// The Messages endpoint for an Anthropic-compatible gateway given in
/// METEOR_ANTHROPIC_BASE_URL. A base URL (`https://gw.example.com`,
/// `…/v1`) gets `/v1/messages` added; a full endpoint is kept. The key goes
/// wherever this points, so it must be https — plain http only to this machine.
pub fn anthropic_endpoint(raw: &str) -> Result<Url, String> {
    const VAR: &str = "METEOR_ANTHROPIC_BASE_URL";
    let mut url = Url::parse(raw.trim()).map_err(|_| format!("{VAR} is not a valid URL."))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_url(&url) => {}
        "http" => return Err(format!("{VAR} must use https:// (plain http only to this computer).")),
        _ => return Err(format!("{VAR} must start with https://.")),
    }
    if !url.username().is_empty() || url.password().is_some() || url.host_str().is_none() {
        return Err(format!("{VAR} must not carry a user name or password."));
    }
    let path = url.path().trim_end_matches('/').to_string();
    let path = if path.ends_with("/v1/messages") {
        path
    } else if path.ends_with("/v1") {
        format!("{path}/messages")
    } else {
        format!("{path}/v1/messages")
    };
    url.set_path(&path);
    url.set_fragment(None);
    Ok(url)
}

// ── Requests that carry a key ─────────────────────────────────────────────────
//
// One rule for every key the app holds (ported from upstream
// `windows/src-tauri/src/net.rs` and `integrations.rs`, pull request 427): it
// goes over https, or over plain http to this machine, and it is never sent on
// to wherever a redirect points. reqwest only strips the standard headers
// (Authorization, Cookie…) when a redirect changes host; a key in a header of
// the service's own (x-api-key, X-N8N-API-KEY) would follow it anywhere.

/// True for an address a key may be sent to: https anywhere, plain http only to
/// this machine.
pub fn key_may_go_to(url: &Url) -> bool {
    url.scheme() == "https" || (url.scheme() == "http" && is_loopback_url(url))
}

/// The address of a service that takes a key, as the user typed it (an n8n
/// instance, any later one): a complete http or https address `key_may_go_to`
/// accepts, without its query and fragment. Otherwise the reason, to show.
/// A `user:password@` stays: an instance behind basic auth needs it.
pub fn keyed_url(raw: &str) -> Result<Url, String> {
    let invalid = || t("Not a valid http:// or https:// address.");
    let mut url = Url::parse(raw.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    if !key_may_go_to(&url) {
        return Err(t("A key is only sent over https, or to a server on this computer."));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// A client for a request that carries a key: refused for an address
/// `key_may_go_to` does not accept, and no redirect is followed (the 3xx comes
/// back as the answer).
pub fn keyed_client(url: &Url, timeout: Duration) -> Result<reqwest::Client, String> {
    if !key_may_go_to(url) {
        return Err(t("A key is only sent over https, or to a server on this computer."));
    }
    builder(url, timeout).redirect(reqwest::redirect::Policy::none()).build().map_err(|e| e.to_string())
}

/// A client for `url`: requests to this machine skip any system proxy, which
/// would otherwise see (and usually fail) a loopback address.
pub fn client(url: &Url, timeout: Duration) -> Result<reqwest::Client, String> {
    builder(url, timeout).build().map_err(|e| e.to_string())
}

fn builder(url: &Url, timeout: Duration) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10).min(timeout))
        .timeout(timeout);
    if is_loopback_url(url) {
        builder = builder.no_proxy();
    }
    // Windows does not resolve `*.localhost` itself: the name goes to the DNS
    // server, which may answer with another machine (a captive portal, a
    // provider that takes over unknown names). Counted as this machine above,
    // such an address skips the proxy and may get a key over plain http, so it
    // has to stay on this machine. (The port given here is ignored: the
    // address's own, or the scheme's, is used.)
    if let Some(host) = url.host_str().filter(|host| host.to_ascii_lowercase().ends_with(".localhost")) {
        builder = builder.resolve_to_addrs(
            host,
            &[SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), SocketAddr::from((Ipv6Addr::LOCALHOST, 0))],
        );
    }
    // A client made for https never follows a redirect down to plain http:
    // reqwest keeps Authorization when host and port stay the same
    // (https://h:8443 to http://h:8443), and a 307 or 308 sends the body again.
    if url.scheme() == "https" {
        builder = builder.https_only(true);
    }
    builder
}

/// One GET for a feature: https only (plain http to this machine), no
/// redirects, `timeout` for the whole exchange, and a body of at most `limit`
/// bytes. Anything but a 2xx answer is an error.
pub async fn bounded_get(url: &str, timeout: Duration, limit: usize) -> Result<Vec<u8>, String> {
    get(url, timeout, limit, false).await
}

/// `bounded_get` for an address that is known to move (a published calendar
/// link, a file on a CDN): up to ten redirects are followed, each of them to
/// https. Plain http is followed only between addresses of this machine.
pub async fn bounded_get_following(url: &str, timeout: Duration, limit: usize) -> Result<Vec<u8>, String> {
    get(url, timeout, limit, true).await
}

/// What a feature may fetch: https anywhere, plain http only from this machine.
pub fn fetchable(url: &str) -> Result<Url, String> {
    let url = Url::parse(url).map_err(|_| tf("Not a valid address: {address}", &[("address", url)]))?;
    if !(url.scheme() == "https" || (url.scheme() == "http" && is_loopback_url(&url))) {
        return Err(t("The address must start with http:// or https://."));
    }
    Ok(url)
}

async fn get(url: &str, timeout: Duration, limit: usize, follow: bool) -> Result<Vec<u8>, String> {
    let url = fetchable(url)?;
    let local = is_loopback_url(&url);
    let redirects = if follow {
        reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() == "https" || (local && is_loopback_url(attempt.url())) {
                attempt.follow()
            } else {
                // Not followed: the 3xx itself is the answer, and that is an error below.
                attempt.stop()
            }
        })
    } else {
        reqwest::redirect::Policy::none()
    };
    let client =
        builder(&url, timeout).redirect(redirects).user_agent(notch_brand::SLUG).build().map_err(|e| e.to_string())?;
    // The address stays out of the error: it can hold a token, and errors are shown and logged.
    let response = client.get(url).send().await.map_err(|e| tf("Network error: {error}", &[("error", &e.without_url().to_string())]))?;
    let status = response.status();
    if !status.is_success() {
        return Err(tf("API error {code}", &[("code", &status.as_u16().to_string())]));
    }
    read_capped(response, limit).await
}

/// The body of `response`, refused past `limit` bytes.
pub async fn read_capped(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(t("The server's answer is too large."));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| tf("Network error: {error}", &[("error", &e.to_string())]))? {
        if body.len() + chunk.len() > limit {
            return Err(t("The server's answer is too large."));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The text of an error body, cut down for a message: `error.message`
/// (OpenAI, Anthropic), a bare `error` string (Ollama), or the first characters.
pub fn error_detail(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(m) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.get("error").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        // Gemini answers some errors as a one-element array.
        if let Some(m) = v.pointer("/0/error/message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    text.trim().chars().take(200).collect()
}

/// Only the host of an address, for the log: never a path, a query or a key.
pub fn host_for_log(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => "?".into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_recognised_and_others_are_not() {
        for host in ["localhost", "LOCALHOST", "app.localhost", "127.0.0.1", "127.8.9.1", "::1", "[::1]", "0.0.0.0", "[::]", "::ffff:127.0.0.1"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["example.com", "192.168.1.10", "10.0.0.2", "localhost.example.com", "[2001:db8::1]", "128.0.0.1", ""] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn a_pasted_server_address_is_cleaned_up() {
        let n = |s: &str| normalise_server_url(s).map(|u| u.to_string());
        assert_eq!(n("  http://localhost:11434/  ").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("http://localhost:11434/v1").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("http://127.0.0.1:1234/v1/").unwrap(), "http://127.0.0.1:1234/");
        assert_eq!(n("localhost:11434/api").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("0.0.0.0:11434").unwrap(), "http://127.0.0.1:11434/");
        assert_eq!(n("http://[::1]:8000").unwrap(), "http://127.0.0.1:8000/");
        assert_eq!(n("https://llm.example.com/proxy/v1?x=1#y").unwrap(), "https://llm.example.com/proxy");
        assert_eq!(n("gpu-box.lan:8000").unwrap(), "http://gpu-box.lan:8000/");
    }

    #[test]
    fn a_server_address_that_is_not_http_is_refused() {
        assert!(normalise_server_url("").is_err());
        assert!(normalise_server_url("   ").is_err());
        assert!(normalise_server_url("file:///etc/passwd").is_err());
        assert!(normalise_server_url("javascript://alert(1)").is_err());
        assert!(normalise_server_url("http://user:pw@example.com").is_err());
        assert!(normalise_server_url("http://").is_err());
    }

    #[test]
    fn join_puts_one_slash_between_base_and_path() {
        let base = normalise_server_url("http://127.0.0.1:11434").unwrap();
        assert_eq!(join(&base, "/v1/models"), "http://127.0.0.1:11434/v1/models");
        let base = Url::parse("https://api.openai.com/v1").unwrap();
        assert_eq!(join(&base, "chat/completions"), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn the_anthropic_gateway_is_taken_as_a_base_url_and_must_be_https() {
        let e = |s: &str| anthropic_endpoint(s).map(|u| u.to_string());
        assert_eq!(e("https://gw.example.com").unwrap(), "https://gw.example.com/v1/messages");
        assert_eq!(e("https://gw.example.com/").unwrap(), "https://gw.example.com/v1/messages");
        assert_eq!(e("https://gw.example.com/anthropic").unwrap(), "https://gw.example.com/anthropic/v1/messages");
        assert_eq!(e("https://gw.example.com/v1").unwrap(), "https://gw.example.com/v1/messages");
        assert_eq!(e("https://gw.example.com/v1/messages/").unwrap(), "https://gw.example.com/v1/messages");
        assert_eq!(e(" http://localhost:4000 ").unwrap(), "http://localhost:4000/v1/messages");
        assert_eq!(e("http://127.0.0.1:4000/v1").unwrap(), "http://127.0.0.1:4000/v1/messages");
        assert!(e("http://gw.example.com").is_err());
        assert!(e("ftp://gw.example.com").is_err());
        assert!(e("https://me:secret@gw.example.com").is_err());
        assert!(e("not a url").is_err());
    }

    #[test]
    fn error_details_come_from_the_usual_places() {
        assert_eq!(error_detail(br#"{"error":{"message":"bad key"}}"#), "bad key");
        assert_eq!(error_detail(br#"{"error":"model 'x' not found"}"#), "model 'x' not found");
        assert_eq!(error_detail(br#"[{"error":{"message":"quota"}}]"#), "quota");
        assert_eq!(error_detail(b"  plain text  "), "plain text");
        assert_eq!(error_detail("x".repeat(500).as_bytes()).len(), 200);
    }

    #[test]
    fn only_the_host_goes_to_the_log() {
        let url = Url::parse("https://gw.example.com:8443/secret/path?key=abc").unwrap();
        assert_eq!(host_for_log(&url), "gw.example.com:8443");
        let url = Url::parse("https://gw.example.com/v1/messages").unwrap();
        assert_eq!(host_for_log(&url), "gw.example.com");
    }

    #[test]
    fn a_key_goes_over_https_or_to_this_computer_only() {
        let base = |raw: &str| keyed_url(raw).map(|u| u.as_str().trim_end_matches('/').to_string());
        assert_eq!(base("https://n8n.example.com").unwrap(), "https://n8n.example.com");
        assert_eq!(base("  https://n8n.example.com/  ").unwrap(), "https://n8n.example.com");
        assert_eq!(base("https://example.com/n8n/").unwrap(), "https://example.com/n8n");
        assert_eq!(base("https://n8n.example.com:8443?x=1#y").unwrap(), "https://n8n.example.com:8443");
        // An instance behind basic auth keeps its credentials.
        assert_eq!(base("https://me:pw@n8n.example.com").unwrap(), "https://me:pw@n8n.example.com");

        for local in ["http://localhost:5678", "http://LOCALHOST:5678/", "http://127.0.0.1:5678", "http://[::1]:5678", "http://n8n.localhost"] {
            assert!(keyed_url(local).is_ok(), "{local}");
        }
        let refused = t("A key is only sent over https, or to a server on this computer.");
        for remote in ["http://n8n.example.com", "http://192.168.1.20:5678", "http://localhost.example.com", "http://[2001:db8::1]:5678"] {
            assert_eq!(keyed_url(remote).unwrap_err(), refused, "{remote}");
            let url = Url::parse(remote).unwrap();
            assert!(!key_may_go_to(&url), "{remote}");
            assert!(keyed_client(&url, Duration::from_secs(5)).is_err(), "{remote}");
        }
        let invalid = t("Not a valid http:// or https:// address.");
        for bad in ["", "n8n.example.com", "localhost:5678", "ftp://n8n.example.com", "file:///C:/n8n", "ws://localhost:5678", "javascript:alert(1)", "https://"] {
            assert_eq!(keyed_url(bad).unwrap_err(), invalid, "{bad:?}");
        }
        // Not http at all, even to this machine.
        assert!(!key_may_go_to(&Url::parse("ftp://localhost/x").unwrap()));
    }

    #[test]
    fn a_request_with_a_key_does_not_follow_a_redirect() {
        // Nothing listens at the target: had the redirect been followed, the
        // request would fail instead of returning the 302.
        let url = serve_once("302 Found", "Location: http://127.0.0.1:9/elsewhere\r\nContent-Length: 0\r\n", vec![]);
        let url = Url::parse(&url).unwrap();
        let response = block_on(async {
            keyed_client(&url, Duration::from_secs(5)).unwrap().get(url.clone()).header("X-N8N-API-KEY", "secret").send().await
        })
        .expect("the 302 itself");
        assert_eq!(response.status().as_u16(), 302);
    }

    #[test]
    fn a_localhost_subdomain_is_reached_on_this_machine_whatever_the_dns_says() {
        // Without the forced resolution, Windows would ask the DNS server for
        // `n8n.localhost`, which does not know it, and the request would fail.
        let served = Url::parse(&serve_once("200 OK", "Content-Length: 2\r\n", b"ok".to_vec())).unwrap();
        let url = Url::parse(&format!("http://n8n.localhost:{}/", served.port().unwrap())).unwrap();
        let response = block_on(async { client(&url, Duration::from_secs(5)).unwrap().get(url.clone()).send().await })
            .expect("the server on this machine");
        assert_eq!(response.status().as_u16(), 200);
    }

    #[test]
    fn a_client_made_for_https_never_goes_back_to_plain_http() {
        let https = Url::parse("https://models.example.com").unwrap();
        let error = block_on(async { client(&https, Duration::from_secs(5)).unwrap().get("http://127.0.0.1:9/").send().await })
            .expect_err("plain http refused");
        // Refused before any connection, not for lack of a server on port 9.
        assert!(error.is_builder(), "{error}");
    }

    /// One-shot HTTP server on a free port answering the next request with `body`.
    pub(crate) fn serve_once(status: &str, headers: &str, body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let head = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n");
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        });
        url
    }

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    #[test]
    fn a_body_past_the_ceiling_is_refused_with_or_without_a_length() {
        let get = |url: String| async move {
            let url = Url::parse(&url).unwrap();
            let response = client(&url, Duration::from_secs(5)).unwrap().get(url).send().await.unwrap();
            read_capped(response, 1000).await
        };
        // Declared length too large.
        let url = serve_once("200 OK", "Content-Length: 5000\r\n", vec![b'x'; 5000]);
        assert!(block_on(get(url)).is_err());
        // No length (read to the end of the connection): stopped while reading.
        let url = serve_once("200 OK", "", vec![b'x'; 5000]);
        assert!(block_on(get(url)).is_err());
        // Under the ceiling: read whole.
        let url = serve_once("200 OK", "Content-Length: 3\r\n", b"abc".to_vec());
        assert_eq!(block_on(get(url)).unwrap(), b"abc");
    }
}
