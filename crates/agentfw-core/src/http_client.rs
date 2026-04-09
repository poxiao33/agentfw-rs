use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::error::{FrameworkError, ModelAdapterError};

/// Build a `reqwest::Client` with `Content-Type: application/json` and an optional
/// `Authorization: Bearer <key>` header.  Used by the OpenAI-compatible adapters.
pub(crate) fn build_bearer_client(api_key: Option<&str>) -> Result<reqwest::Client, FrameworkError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key {
        let key = key.trim();
        if key.is_empty() {
            return Err(FrameworkError::from(ModelAdapterError::Request(
                "api key must not be empty".to_string(),
            )));
        }
        let auth = format!("Bearer {key}");
        let value = HeaderValue::from_str(&auth)
            .map_err(|err| ModelAdapterError::Request(format!("invalid api key header (contains non-ASCII characters): {err}")))?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|err| ModelAdapterError::Request(format!("failed to build reqwest client: {err}")))?)
}
