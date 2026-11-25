//! The only file that touches the network: a blocking `reqwest` client with rustls and bundled root certificates
//! (so Android and iOS need no platform verifier). Redirects are never followed here.
use crate::failure::AuthFailure;
use crate::http::{CookieMode, Http, HttpRequest, HttpResponse, Method};
use reqwest::blocking::Client;
use reqwest::cookie::Jar;
use reqwest::redirect::Policy;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// Largest response body accepted. The login pages are small; anything bigger is refused.
pub const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// One HTTP client, one connection pool. Cookies are handled by hand so that a request can be sent with the
/// session's cookies (`CookieMode::Jar`) or with none (`CookieMode::None`) over the SAME pooled connections:
/// the cookie-free bearer check no longer pays a second TCP+TLS handshake, and it still carries no cookies.
#[derive(Clone)]
pub struct ReqwestHttp {
    client: Client,
    jar: Arc<Jar>,
}

impl ReqwestHttp {
    pub fn new(timeout: Duration) -> Result<Self, AuthFailure> {
        let client = Client::builder()
            .redirect(Policy::none())
            .cookie_store(false)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .build()
            .map_err(|_| AuthFailure::Network)?;
        Ok(Self {
            client,
            jar: Arc::new(Jar::default()),
        })
    }
}

#[cfg(feature = "dev-net")]
impl ReqwestHttp {
    /// DEV/TEST ONLY (feature `dev-net`, off by default, never in a release build). Same client as [`Self::new`]
    /// plus: `root_ca_pem` is trusted IN ADDITION to the bundled roots, and each host in `host_map` is connected
    /// to the given address on port 443 instead of being looked up in DNS. TLS verification stays on (the
    /// certificate must still match the host name); redirects are still never followed.
    pub fn with_dev_net(
        root_ca_pem: &[u8],
        host_map: &std::collections::BTreeMap<String, std::net::IpAddr>,
        timeout: Duration,
    ) -> Result<Self, AuthFailure> {
        let ca =
            reqwest::Certificate::from_pem(root_ca_pem).map_err(|_| AuthFailure::InvalidInput)?;
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .cookie_store(false)
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .add_root_certificate(ca);
        for (host, ip) in host_map {
            builder = builder.resolve(host, std::net::SocketAddr::new(*ip, 443));
        }
        let client = builder.build().map_err(|_| AuthFailure::Network)?;
        Ok(Self {
            client,
            jar: Arc::new(Jar::default()),
        })
    }
}

#[cfg(feature = "dev-session-export")]
impl ReqwestHttp {
    /// DEV/TEST ONLY (feature `dev-session-export`, off by default, never in the CLI or any app build).
    /// Returns `(name, value)` for the cookies the jar would send to `url`. These are live session
    /// secrets: the caller must write them only to a private file and delete it after use.
    pub fn dev_export_cookies(&self, url: &url::Url) -> Vec<(String, String)> {
        use reqwest::cookie::CookieStore;
        let Some(h) = self.jar.cookies(url) else {
            return vec![];
        };
        let Ok(h) = h.to_str() else { return vec![] };
        h.split("; ")
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
}

impl Http for ReqwestHttp {
    /// Opens (and pools) a connection to the host with a bodiless HEAD request that carries no cookies,
    /// no credentials and no session state, on a background thread. Failures are ignored on purpose.
    fn prewarm(&mut self, url: &Url) {
        let client = self.client.clone();
        let url = url.clone();
        std::thread::spawn(move || {
            let _ = client.head(url).timeout(Duration::from_secs(10)).send();
        });
    }

    fn cookie_header(&self, url: &Url) -> Option<zeroize::Zeroizing<String>> {
        use reqwest::cookie::CookieStore;
        let v = self.jar.cookies(url)?;
        v.to_str()
            .ok()
            .map(|s| zeroize::Zeroizing::new(s.to_string()))
    }

    fn send(&mut self, request: HttpRequest) -> Result<HttpResponse, AuthFailure> {
        use reqwest::cookie::CookieStore;
        let use_jar = request.cookies == CookieMode::Jar;
        let cookie = if use_jar {
            self.jar.cookies(&request.url)
        } else {
            None
        };
        let (response, set_cookies) = self.exchange(&request, cookie)?;
        if use_jar {
            for v in &set_cookies {
                self.jar.add_cookie_str(v, &request.url);
            }
        }
        Ok(response)
    }

    /// Never reads or writes `self.jar`: the caller owns the cookies (see the trait documentation).
    fn send_detached(
        &mut self,
        request: HttpRequest,
    ) -> Result<(HttpResponse, Vec<zeroize::Zeroizing<String>>), AuthFailure> {
        self.exchange(&request, None)
    }

    /// Never reads or writes `self.jar`; redirects are not followed (client policy `none`).
    fn send_raw(
        &mut self,
        method: &str,
        url: &Url,
        headers: &[(String, zeroize::Zeroizing<String>)],
        body: Option<&[u8]>,
        timeout: Option<Duration>,
        max_body_bytes: u64,
    ) -> Result<crate::http::RawExchange, AuthFailure> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| AuthFailure::InvalidInput)?;
        let mut b = self.client.request(method, url.clone());
        for (name, value) in headers {
            b = b.header(name.as_str(), value.as_str());
        }
        if let Some(body) = body {
            b = b.body(body.to_vec());
        }
        if let Some(timeout) = timeout {
            b = b.timeout(timeout);
        }
        let resp = b.send().map_err(|e| {
            if e.is_timeout() {
                AuthFailure::Timeout
            } else if e.is_builder() {
                AuthFailure::InvalidInput
            } else {
                AuthFailure::Network
            }
        })?;
        let status = resp.status().as_u16();
        let response_headers = resp
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    zeroize::Zeroizing::new(String::from_utf8_lossy(value.as_bytes()).into_owned()),
                )
            })
            .collect();
        let mut raw = Vec::new();
        resp.take(max_body_bytes.saturating_add(1))
            .read_to_end(&mut raw)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut {
                    AuthFailure::Timeout
                } else {
                    AuthFailure::Network
                }
            })?;
        if raw.len() as u64 > max_body_bytes {
            return Err(AuthFailure::UnexpectedPage);
        }
        Ok((status, response_headers, raw))
    }
}

impl ReqwestHttp {
    /// One request/response exchange. `cookie` is the session jar's `Cookie` header, if the caller wants it
    /// sent. Returns the response and its `Set-Cookie` header values (not stored anywhere here).
    fn exchange(
        &self,
        request: &HttpRequest,
        cookie: Option<reqwest::header::HeaderValue>,
    ) -> Result<(HttpResponse, Vec<zeroize::Zeroizing<String>>), AuthFailure> {
        let mut b = match request.method {
            Method::Get => self.client.get(request.url.clone()),
            Method::Post => self.client.post(request.url.clone()),
        };
        if let Some(c) = cookie {
            b = b.header(reqwest::header::COOKIE, c);
        }
        if let Some(ct) = request.content_type {
            b = b.header("Content-Type", ct);
        }
        for (name, value) in &request.headers {
            b = b.header(*name, value.as_str());
        }
        if let Some(body) = &request.body {
            b = b.body(body.as_str().to_owned());
        }
        let resp = b.send().map_err(|e| {
            if e.is_timeout() {
                AuthFailure::Timeout
            } else {
                AuthFailure::Network
            }
        })?;
        let set_cookies = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(|v| zeroize::Zeroizing::new(v.to_string()))
            .collect();
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let (location, content_type) = (header("location"), header("content-type"));
        let mut raw = Vec::new();
        resp.take(MAX_BODY_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|_| AuthFailure::Network)?;
        if raw.len() as u64 > MAX_BODY_BYTES {
            return Err(AuthFailure::UnexpectedPage);
        }
        Ok((
            HttpResponse {
                status,
                location,
                content_type,
                body: String::from_utf8_lossy(&raw).into_owned(),
            },
            set_cookies,
        ))
    }
}
