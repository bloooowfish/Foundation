use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

const INDEX_HTML: &str = include_str!("web/index.html");
const DASHBOARD_CSS: &str = include_str!("web/dashboard.css");
const DASHBOARD_JS: &str = include_str!("web/dashboard.js");

fn static_asset_response(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn index_handler() -> Response {
    static_asset_response(INDEX_HTML, "text/html; charset=utf-8")
}

async fn css_handler() -> Response {
    static_asset_response(DASHBOARD_CSS, "text/css; charset=utf-8")
}

async fn js_handler() -> Response {
    static_asset_response(DASHBOARD_JS, "application/javascript; charset=utf-8")
}

/// Serves the embedded Housing Studio web dashboard from `:21064/`.
pub fn router() -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/dashboard.css", get(css_handler))
        .route("/dashboard.js", get(js_handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn web_ui_handlers_return_expected_content_types() {
        let html = index_handler().await;
        assert_eq!(html.status(), StatusCode::OK);
        assert_eq!(
            html.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        assert!(!INDEX_HTML.is_empty());

        let css = css_handler().await;
        assert_eq!(css.status(), StatusCode::OK);
        assert_eq!(
            css.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/css; charset=utf-8"
        );
        assert!(!DASHBOARD_CSS.is_empty());

        let js = js_handler().await;
        assert_eq!(js.status(), StatusCode::OK);
        assert_eq!(
            js.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/javascript; charset=utf-8"
        );
        assert!(!DASHBOARD_JS.is_empty());
    }
}
