use crate::failure::AuthFailure;
use url::Url;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// Which cookies a request may use. The bearer check must run with `None` so it proves the token works on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieMode {
    /// The session's cookie jar (sent and updated).
    Jar,
    /// No cookies sent, none stored.
    None,
}

/// One request. The body and headers can hold secrets (password, bearer, CSRF token), so `Debug` hides them.
pub struct HttpRequest {
    pub method: Method,
    pub url: Url,
    pub body: Option<Zeroizing<String>>,
    pub content_type: Option<&'static str>,
    pub headers: Vec<(&'static str, Zeroizing<String>)>,
    pub cookies: CookieMode,
    /// Fixed label of the flow step this request belongs to (for timing logs only; never a secret).
    pub stage: &'static str,
}

impl HttpRequest {
    pub fn get(url: Url) -> Self {
        Self {
            method: Method::Get,
            url,
            body: None,
            content_type: None,
            headers: vec![],
            cookies: CookieMode::Jar,
            stage: "",
        }
    }
    pub fn post(url: Url, content_type: &'static str, body: String) -> Self {
        Self {
            method: Method::Post,
            url,
            body: Some(Zeroizing::new(body)),
            content_type: Some(content_type),
            headers: vec![],
            cookies: CookieMode::Jar,
            stage: "",
        }
    }
    pub fn with_header(mut self, name: &'static str, value: String) -> Self {
        self.headers.push((name, Zeroizing::new(value)));
        self
    }
    pub fn stage(mut self, stage: &'static str) -> Self {
        self.stage = stage;
        self
    }
    pub fn without_cookies(mut self) -> Self {
        self.cookies = CookieMode::None;
        self
    }
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HttpRequest({:?} {}{}, [body/headers hidden])",
            self.method,
            self.url.host_str().unwrap_or("?"),
            self.url.path()
        )
    }
}

/// One response. Redirects are never followed by the transport; the flow decides.
pub struct HttpResponse {
    pub status: u16,
    pub location: Option<String>,
    pub content_type: Option<String>,
    pub body: String,
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HttpResponse({}, [body hidden])", self.status)
    }
}

impl HttpResponse {
    pub fn is_json(&self) -> bool {
        self.content_type
            .as_deref()
            .map(str::to_ascii_lowercase)
            .is_some_and(|c| c.contains("application/json") || c.contains("+json"))
    }
}

/// A blocking HTTP client with one cookie jar per instance. Implementations must not follow redirects.
pub trait Http {
    /// Optional hint: open a connection to `url` in the background so a later request to that host is faster.
    /// It must send no credentials, cookies or session state. The default does nothing.
    fn prewarm(&mut self, _url: &Url) {}
    /// The `Cookie` header value the session jar would send to `url`, for the one caller that must hand a
    /// session cookie to a private consumer (Navigate). Secret: never log it. The default has no jar.
    fn cookie_header(&self, _url: &Url) -> Option<Zeroizing<String>> {
        None
    }
    fn send(&mut self, request: HttpRequest) -> Result<HttpResponse, AuthFailure>;
    /// For a caller that keeps its OWN cookie jars (the proto engine): send `request` without touching this
    /// instance's jar (as with [`CookieMode::None`], whatever `request.cookies` says) and also return the raw
    /// `Set-Cookie` header values of the response. The caller adds its own `Cookie` header to `request.headers`.
    /// The returned values are session secrets: never log them. The default (for implementations that cannot
    /// see response headers) sends cookie-free and reports no `Set-Cookie` values.
    fn send_detached(
        &mut self,
        mut request: HttpRequest,
    ) -> Result<(HttpResponse, Vec<Zeroizing<String>>), AuthFailure> {
        request.cookies = CookieMode::None;
        self.send(request).map(|r| (r, Vec::new()))
    }
    /// Additive hook for the proto engine's v2 `net.start` (any method token, any header set, binary bodies).
    /// Like [`Self::send_detached`] it never touches this instance's jar and never follows a redirect. The
    /// caller supplies every request header (including `Cookie` and `Content-Type`). Returns the status, ALL
    /// response headers in wire order with repeated fields preserved (names lower-case; `set-cookie` values
    /// are session secrets) and the body bytes after any content decoding the transport does itself. A body
    /// longer than `max_body_bytes` is refused with [`AuthFailure::UnexpectedPage`]. `timeout` overrides the
    /// client's timeout for this request. The default (for implementations without raw access) refuses.
    fn send_raw(
        &mut self,
        _method: &str,
        _url: &Url,
        _headers: &[(String, Zeroizing<String>)],
        _body: Option<&[u8]>,
        _timeout: Option<std::time::Duration>,
        _max_body_bytes: u64,
    ) -> Result<RawExchange, AuthFailure> {
        Err(AuthFailure::InvalidInput)
    }
}

/// What [`Http::send_raw`] returns: status, response headers (lower-case names, repeats kept), body bytes.
pub type RawExchange = (u16, Vec<(String, Zeroizing<String>)>, Vec<u8>);
