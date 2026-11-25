//! The dashboard: three static files embedded into the binary (no separate
//! web server, nothing to install). `web_root` can override them for dev.

use std::borrow::Cow;

use axum::{
    extract::State,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};

use crate::{util::ApiError, Shared};

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
    (
        [
            (header::CONTENT_TYPE, ctype),
            (header::CACHE_CONTROL, "no-cache"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        body,
    )
        .into_response()
}
