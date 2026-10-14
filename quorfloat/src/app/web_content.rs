//! URL boundaries for the conversation WebView and the system browser.

use tauri::Url;

/// Only absolute HTTP(S) addresses can be opened by the browser command.
pub fn external_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "invalid external URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("external URL must use HTTP or HTTPS".to_owned());
    }
    Ok(url)
}

/// Allow the panel document and its fragments, never a different page.
pub fn allows_navigation(url: &Url, dev_url: Option<&Url>) -> bool {
    if let Some(dev) = dev_url {
        return url.origin() == dev.origin() && url.path() == dev.path() && url.query() == dev.query();
    }
    let embedded = match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost") && url.port().is_none(),
        _ => false,
    };
    // Tauri simplifies index.html to tauri://localhost, whose path is empty.
    embedded && matches!(url.path(), "" | "/" | "/index.html") && url.query().is_none()
}
