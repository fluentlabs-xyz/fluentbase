use crate::FetchError;
use std::{io::Read as _, time::Duration};

/// Downloads `url` with a blocking client, refusing responses larger than `max_bytes`.
///
/// `404 Not Found` and `410 Gone` are reported through [`FetchError::is_not_found`], allowing a
/// caller to distinguish an absent optional release asset from a transient transport failure.
pub fn bounded_http_fetch(
    user_agent: &str,
    timeout: Duration,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, FetchError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(user_agent)
        .timeout(timeout)
        .build()
        .map_err(|err| FetchError::new(format!("failed to build HTTP client: {err}")))?;
    let response = client
        .get(url)
        .send()
        .map_err(|err| FetchError::new(format!("GET {url}: {err}")))?;
    if matches!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
    ) {
        return Err(FetchError::not_found(format!(
            "GET {url} returned {}",
            response.status()
        )));
    }
    let response = response
        .error_for_status()
        .map_err(|err| FetchError::new(format!("GET {url} returned non-success: {err}")))?;

    if let Some(len) = response.content_length() {
        if len > max_bytes as u64 {
            return Err(FetchError::new(format!(
                "advertises {len} bytes, over the {max_bytes} byte limit"
            )));
        }
    }

    let mut bytes = Vec::new();
    response
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| FetchError::new(format!("reading response body: {err}")))?;
    if bytes.len() > max_bytes {
        return Err(FetchError::new(format!(
            "exceeds the {max_bytes} byte limit"
        )));
    }
    Ok(bytes)
}
