//! More than one server pod (docs/SLICES.md §8, docair-702.32): an upload is kept in the pod
//! that took it, its id names that pod, and another pod forwards the upload's requests there,
//! only to addresses the server pods' own name resolves to.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// A forwarded request carries this header, and is never forwarded again.
pub const FORWARDED: &str = "x-docair-forwarded";

/// A forwarded request gets this long for its answer: as long as a question to the model.
const WAIT: Duration = Duration::from_secs(300);

/// The largest answer forwarded: a page of a big project is a few MB.
const MAX_ANSWER: u64 = 256 * 1024 * 1024;

/// This pod's address and the name that resolves to every server pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peers {
    /// `ip:port`, as the other pods reach this one.
    pub me: String,
    /// `host:port` that resolves to every server pod (`DOCAIR_PEERS`, a headless Service).
    pub lookup: String,
}

impl Peers {
    /***
     * The pods' settings: this pod's IP from `DOCAIR_POD_IP` (the downward API), the name of all server pods from `DOCAIR_PEERS`, whose port every pod listens on; none when either is unset (one pod, or `docair serve`).
     ***/
    pub fn from_env() -> Option<Peers> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Peers::of(&var("DOCAIR_POD_IP")?, &var("DOCAIR_PEERS")?)
    }

    /// This pod at `ip`, the server pods at `lookup` (`host:port`, the port all of them listen on).
    pub fn of(ip: &str, lookup: &str) -> Option<Peers> {
        let ip: std::net::IpAddr = ip.trim().parse().ok()?;
        let port: u16 = lookup.rsplit_once(':')?.1.parse().ok()?;
        Some(Peers {
            me: SocketAddr::new(ip, port).to_string(),
            lookup: lookup.trim().to_string(),
        })
    }

    /// The pod part of the ids this pod gives: its address in hex.
    pub fn prefix(&self) -> String {
        self.me.bytes().map(|b| format!("{b:02x}")).collect()
    }
}

/// The pod an upload id names, `ip:port`; none for an id without one.
pub fn pod_of(id: &str) -> Option<String> {
    let (hex, _) = id.split_once('-')?;
    if hex.is_empty() || hex.len() % 2 != 0 || hex.len() > 120 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// Every address the server pods' name resolves to.
async fn server_pods(lookup: &str) -> HashSet<SocketAddr> {
    tokio::net::lookup_host(lookup)
        .await
        .map(|a| a.collect())
        .unwrap_or_default()
}

/***
 * Forward a request for an upload this pod does not keep to the pod `to` that keeps it: same method, path, query, body, `content-type` and `accept`; its answer as it is. None when `to` is not a server pod or does not answer, so the upload counts as gone.
 *
 * @calls docair-server.kept_upload (HTTP to the pod in the id, port 8080)
 ***/
pub async fn forward(peers: &Peers, to: &str, req: Request, max_body: usize) -> Option<Response> {
    let addr: SocketAddr = to.parse().ok()?;
    if to == peers.me || !server_pods(&peers.lookup).await.contains(&addr) {
        return None;
    }
    let method = req.method().clone();
    if method != Method::GET && method != Method::POST {
        return None;
    }
    let path = req.uri().path_and_query().map(|p| p.as_str().to_string())?;
    let (content_type, accept) = {
        let h = req.headers();
        let of = |name: header::HeaderName| {
            h.get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        (of(header::CONTENT_TYPE), of(header::ACCEPT))
    };
    let body = axum::body::to_bytes(req.into_body(), max_body).await.ok()?;
    let url = format!("http://{addr}{path}");
    let answer = tokio::task::spawn_blocking(move || -> Option<(u16, Option<String>, Vec<u8>)> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(WAIT))
            .http_status_as_error(false)
            .proxy(None)
            .build()
            .into();
        let mut res = if method == Method::GET {
            let mut r = agent.get(&url).header(FORWARDED, "1");
            if let Some(a) = &accept {
                r = r.header("accept", a);
            }
            r.call().ok()?
        } else {
            let mut r = agent.post(&url).header(FORWARDED, "1");
            if let Some(a) = &accept {
                r = r.header("accept", a);
            }
            if let Some(c) = &content_type {
                r = r.header("content-type", c);
            }
            r.send(&body[..]).ok()?
        };
        let status = res.status().as_u16();
        let kind = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = res
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_vec()
            .ok()?;
        Some((status, kind, bytes))
    })
    .await
    .ok()??;
    let (status, kind, bytes) = answer;
    let mut res = (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        Body::from(bytes),
    )
        .into_response();
    if let Some(k) = kind.and_then(|k| HeaderValue::from_str(&k).ok()) {
        res.headers_mut().insert(header::CONTENT_TYPE, k);
    }
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_names_its_pod() {
        let p = Peers {
            me: "10.244.0.7:8080".into(),
            lookup: "docair-server-peers:8080".into(),
        };
        let id = format!("{}-0123456789abcdef0123456789abcdef", p.prefix());
        assert_eq!(pod_of(&id).as_deref(), Some("10.244.0.7:8080"));
        assert_eq!(
            pod_of("0123456789abcdef0123456789abcdef"),
            None,
            "an id of one pod names none"
        );
        assert_eq!(pod_of("zz-0123"), None);
        assert_eq!(pod_of("abc-0123"), None, "an odd number of digits");
        let p = Peers::of("fd00::7", "docair-server-peers:8080").unwrap();
        assert_eq!(p.me, "[fd00::7]:8080", "an IPv6 pod, as a lookup gives it");
        assert_eq!(Peers::of("10.0.0.1", "no-port"), None);
    }
}
