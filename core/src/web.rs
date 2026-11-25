//! The dashboard (embedded static files), security headers, CSRF guard and
//! the HTTP → HTTPS redirect.

use std::borrow::Cow;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    Router,
};

use crate::{util::ApiError, Shared};

/// Request extension: true when the request arrived over TLS.
#[derive(Clone, Copy)]
pub struct Secure(pub bool);

fn embedded(name: &str) -> &'static str {
    match name {
        "app.js" => include_str!("../../frontend/app.js"),
        "style.css" => include_str!("../../frontend/style.css"),
        "favicon.svg" => include_str!("../../frontend/favicon.svg"),
        _ => include_str!("../../frontend/index.html"),
    }
}

const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; \
                   connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub async fn fallback(State(st): State<Shared>, uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") {
        return ApiError::new(StatusCode::NOT_FOUND, "Không có API này").into_response();
    }
    let (name, ctype) = match path {
        "/app.js" => ("app.js", "text/javascript; charset=utf-8"),
        "/style.css" => ("style.css", "text/css; charset=utf-8"),
        "/favicon.svg" => ("favicon.svg", "image/svg+xml"),
        _ => ("index.html", "text/html; charset=utf-8"),
    };
    let body: Cow<'static, str> = match &st.cfg.web_root {
        Some(root) => std::fs::read_to_string(root.join(name))
            .map(Cow::Owned)
            .unwrap_or_else(|_| Cow::Borrowed(embedded(name))),
        None => Cow::Borrowed(embedded(name)),
    };
    ([(header::CONTENT_TYPE, ctype), (header::CACHE_CONTROL, "no-cache"), (header::CONTENT_SECURITY_POLICY, CSP)], body)
        .into_response()
}

/// Security headers on every response.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let secure = req.extensions().get::<Secure>().is_some_and(|s| s.0);
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if api && !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if secure {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    }
    res
}

/// Mutating API calls must carry `X-RustNAS: 1`. Browsers cannot add custom
/// headers to cross-site requests without a CORS preflight (which we never
/// answer), so this blocks CSRF even beyond SameSite=Strict cookies.
pub async fn csrf_guard(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && req.uri().path().starts_with("/api/") && req.headers().get("x-rustnas").is_none_or(|v| v != "1") {
        return ApiError::new(StatusCode::FORBIDDEN, "Yêu cầu bị chặn (thiếu header X-RustNAS)").into_response();
    }
    next.run(req).await
}

fn clean_host(headers: &HeaderMap) -> String {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let host = if let Some(rest) = host.strip_prefix('[') {
        format!("[{}]", rest.split(']').next().unwrap_or(""))
    } else {
        host.split(':').next().unwrap_or("").to_string()
    };
    let ok = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-[]:".contains(c));
    if ok { host } else { "localhost".into() }
}

/// HTTP listener when HTTPS is on: everything redirects to HTTPS.
pub fn redirect_router(https_port: u16) -> Router {
    Router::new().fallback(move |headers: HeaderMap, uri: Uri| async move {
        let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        let port = if https_port == 443 { String::new() } else { format!(":{https_port}") };
        Redirect::permanent(&format!("https://{}{port}{pq}", clean_host(&headers)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_cleaning() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "192.168.1.5:8080".parse().unwrap());
        assert_eq!(clean_host(&h), "192.168.1.5");
        h.insert(header::HOST, "[fe80::1]:8080".parse().unwrap());
        assert_eq!(clean_host(&h), "[fe80::1]");
        h.insert(header::HOST, "evil.com/x".parse().unwrap());
        assert_eq!(clean_host(&h), "localhost");
    }
}
