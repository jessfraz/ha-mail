use crate::models::{Environment, MailItem, Ocr, valid_id};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;

const MAX_JSON: usize = 8 * 1024 * 1024;
pub const MAX_DOCUMENT: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct Stable {
    client: Client,
    origin: Url,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{code}")]
pub struct ProviderError {
    pub code: &'static str,
    pub retry_after: Option<u64>,
}
impl ProviderError {
    fn new(code: &'static str) -> Self {
        Self {
            code,
            retry_after: None,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection<T> {
    pub edges: Vec<Edge<T>>,
    pub page_info: PageInfo,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Edge<T> {
    pub node: T,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub end_cursor: Option<String>,
    pub has_next_page: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub line1: String,
    #[serde(default)]
    pub line2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Location {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    pub address: Address,
    #[serde(default)]
    pub onboarding: Option<Onboarding>,
}
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Onboarding {
    pub status: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct LocationRef {
    pub id: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderItem {
    pub id: String,
    #[serde(rename = "from")]
    pub sender: String,
    pub location: LocationRef,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub read_at: Option<String>,
    #[serde(default)]
    pub clear_at: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub scan_details: Option<Scan>,
    #[serde(default)]
    pub shred_details: Option<StatusDetail>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct StatusDetail {
    pub status: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    #[serde(default)]
    pub image_url: Option<String>,
    pub status: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub scan_notice_type: Option<String>,
    #[serde(default)]
    pub ocr_result_urls: Option<Vec<String>>,
}

impl ProviderItem {
    pub fn snapshot(&self, now: i64) -> Result<MailItem, ProviderError> {
        if !valid_id(&self.id)
            || !valid_id(&self.location.id)
            || self.sender.len() > 4000
            || self
                .scan_details
                .as_ref()
                .and_then(|s| s.summary.as_ref())
                .is_some_and(|s| s.len() > 16_000)
        {
            return Err(ProviderError::new("invalid_provider_item"));
        }
        for at in [
            &self.created_at,
            &self.archived_at,
            &self.read_at,
            &self.clear_at,
        ]
        .into_iter()
        .flatten()
        {
            if chrono::DateTime::parse_from_rfc3339(at).is_err() {
                return Err(ProviderError::new("invalid_provider_timestamp"));
            }
        }
        Ok(MailItem {
            id: self.id.clone(),
            sender: self.sender.clone(),
            received_at: self.created_at.clone(),
            first_seen_at: now,
            observed_at: now,
            archived_at: self.archived_at.clone(),
            read_at: self.read_at.clone(),
            clear_at: self.clear_at.clone(),
            scan_status: self
                .scan_details
                .as_ref()
                .map_or("none", |s| s.status.as_str())
                .into(),
            scan_notice: self
                .scan_details
                .as_ref()
                .and_then(|s| s.scan_notice_type.clone()),
            shred_status: self.shred_details.as_ref().map(|s| s.status.clone()),
            summary: self.scan_details.as_ref().and_then(|s| s.summary.clone()),
            has_envelope: self.image_url.as_ref().is_some_and(|s| !s.is_empty()),
            has_pdf: self
                .scan_details
                .as_ref()
                .and_then(|s| s.image_url.as_ref())
                .is_some_and(|s| !s.is_empty()),
            has_ocr: self
                .scan_details
                .as_ref()
                .and_then(|s| s.ocr_result_urls.as_ref())
                .is_some_and(|s| !s.is_empty()),
            revision: 1,
        })
    }
}

pub struct Document {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

impl Stable {
    pub fn new(key: &str, environment: Environment) -> Result<Self, ProviderError> {
        Self::with_origin(key, environment.origin())
    }
    pub fn with_origin(key: &str, origin: &str) -> Result<Self, ProviderError> {
        if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(ProviderError::new("invalid_provider_credential"));
        }
        let mut token = HeaderValue::from_str(key)
            .map_err(|_| ProviderError::new("invalid_provider_credential"))?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", token);
        let builder = || {
            Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
        };
        Ok(Self {
            client: builder()
                .default_headers(headers)
                .build()
                .map_err(|_| ProviderError::new("provider_client_failed"))?,
            origin: Url::parse(origin)
                .map_err(|_| ProviderError::new("invalid_provider_origin"))?,
        })
    }
    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ProviderError> {
        let url = self
            .origin
            .join(path)
            .map_err(|_| ProviderError::new("invalid_provider_path"))?;
        let response = self
            .client
            .get(url)
            .query(query)
            .send()
            .await
            .map_err(|_| ProviderError::new("provider_unavailable"))?;
        let bytes = bounded(response, MAX_JSON).await?;
        serde_json::from_slice(&bytes).map_err(|_| ProviderError::new("invalid_provider_response"))
    }
    pub async fn locations(
        &self,
        cursor: Option<&str>,
    ) -> Result<Connection<Location>, ProviderError> {
        let mut query = vec![("first", "100".into())];
        if let Some(cursor) = cursor {
            query.push(("after", cursor.into()));
        }
        self.get("/v1/locations", &query).await
    }
    pub async fn location(&self, id: &str) -> Result<Location, ProviderError> {
        if !valid_id(id) {
            return Err(ProviderError::new("invalid_location"));
        }
        // A filtered list is the published location lookup contract.
        let result: Connection<Location> = self
            .get("/v1/locations", &[("id", id.into()), ("first", "2".into())])
            .await?;
        let mut found = result.edges.into_iter().filter(|e| e.node.id == id);
        let location = found
            .next()
            .ok_or_else(|| ProviderError::new("location_not_found"))?
            .node;
        if found.next().is_some() {
            return Err(ProviderError::new("invalid_provider_response"));
        }
        Ok(location)
    }
    pub async fn page(
        &self,
        location: &str,
        archived: bool,
        cursor: Option<&str>,
    ) -> Result<Connection<ProviderItem>, ProviderError> {
        let mut query = vec![
            ("locationId", location.into()),
            ("archived", archived.to_string()),
            ("first", "100".into()),
        ];
        if let Some(cursor) = cursor {
            query.push(("after", cursor.into()));
        }
        self.get("/v1/mail-items", &query).await
    }
    pub async fn item(&self, id: &str, location: &str) -> Result<ProviderItem, ProviderError> {
        if !valid_id(id) {
            return Err(ProviderError::new("invalid_item_id"));
        }
        let item: ProviderItem = self.get(&format!("/v1/mail-items/{id}"), &[]).await?;
        if item.id != id || item.location.id != location {
            return Err(ProviderError::new("provider_location_mismatch"));
        }
        Ok(item)
    }
    async fn asset(&self, url: &str, maximum: usize) -> Result<Vec<u8>, ProviderError> {
        let url = Url::parse(url).map_err(|_| ProviderError::new("invalid_document_url"))?;
        let host = url
            .host_str()
            .ok_or_else(|| ProviderError::new("invalid_document_url"))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some_and(|p| p != 443)
            || url.fragment().is_some()
        {
            return Err(ProviderError::new("invalid_document_url"));
        }
        // Stable does not publish a fixed asset-host list. Accept its fresh,
        // item-bound HTTPS URLs, but pin validated public DNS answers so a
        // signed URL cannot become a route into HA or cloud metadata services.
        let addresses =
            tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 443)))
                .await
                .map_err(|_| ProviderError::new("document_unavailable"))?
                .map_err(|_| ProviderError::new("document_unavailable"))?
                .collect::<Vec<_>>();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|address| !public_address(address.ip()))
        {
            return Err(ProviderError::new("document_host_not_allowed"));
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(|_| ProviderError::new("document_unavailable"))?;
        // This separate client has no Stable credential or inherited proxy.
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|_| ProviderError::new("document_unavailable"))?;
        bounded(response, maximum).await
    }
    pub async fn document(
        &self,
        item: &ProviderItem,
        pdf: bool,
    ) -> Result<Document, ProviderError> {
        let url = if pdf {
            item.scan_details
                .as_ref()
                .and_then(|s| s.image_url.as_deref())
        } else {
            item.image_url.as_deref()
        }
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProviderError::new("document_not_ready"))?;
        let bytes = self.asset(url, MAX_DOCUMENT).await?;
        let content_type = if pdf && bytes.starts_with(b"%PDF-") {
            "application/pdf"
        } else if !pdf && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if !pdf && bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else {
            return Err(ProviderError::new("invalid_document_type"));
        };
        Ok(Document {
            bytes,
            content_type,
        })
    }
    pub async fn ocr(&self, item: &ProviderItem) -> Result<Ocr, ProviderError> {
        #[derive(Deserialize)]
        struct Textract {
            #[serde(rename = "Blocks")]
            blocks: Vec<Block>,
        }
        #[derive(Deserialize)]
        struct Block {
            #[serde(rename = "BlockType")]
            kind: String,
            #[serde(rename = "Text")]
            text: Option<String>,
        }
        let mut pages = Vec::new();
        let urls = item
            .scan_details
            .as_ref()
            .and_then(|s| s.ocr_result_urls.as_ref())
            .ok_or_else(|| ProviderError::new("ocr_not_ready"))?;
        if urls.len() > 100 {
            return Err(ProviderError::new("ocr_too_large"));
        }
        let mut total = 0;
        for url in urls {
            let bytes = self.asset(url, 1024 * 1024).await?;
            total += bytes.len();
            if total > MAX_JSON {
                return Err(ProviderError::new("ocr_too_large"));
            }
            let result: Textract = serde_json::from_slice(&bytes)
                .map_err(|_| ProviderError::new("invalid_ocr_response"))?;
            pages.push(
                result
                    .blocks
                    .into_iter()
                    .filter(|b| b.kind == "LINE")
                    .filter_map(|b| b.text)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        Ok(Ocr { pages })
    }
}

async fn bounded(
    mut response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, ProviderError> {
    if !response.status().is_success() {
        let code = match response.status().as_u16() {
            401 | 403 => "provider_authentication_required",
            404 => "provider_not_found",
            429 => "provider_rate_limited",
            _ => "provider_unavailable",
        };
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.clamp(60, 3600));
        return Err(ProviderError { code, retry_after });
    }
    if response
        .content_length()
        .is_some_and(|n| n > maximum as u64)
    {
        return Err(ProviderError::new("provider_response_too_large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderError::new("provider_unavailable"))?
    {
        if bytes.len() + chunk.len() > maximum {
            return Err(ProviderError::new("provider_response_too_large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn public_address(address: std::net::IpAddr) -> bool {
    match address.to_canonical() {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b))
                && !(a == 192 && b == 0 && c == 0)
        }
        std::net::IpAddr::V6(ip) => {
            let parts = ip.segments();
            parts[0] & 0xe000 == 0x2000
                && parts[0] != 0x2002
                && !(parts[0] == 0x2001 && matches!(parts[1], 0 | 2 | 0x10..=0x2f | 0xdb8))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::stable::{Stable, public_address};
    #[test]
    fn signed_asset_destinations_exclude_local_metadata_and_mapped_ranges() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "198.18.0.1",
            "192.0.2.1",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_address(address.parse().unwrap()));
        }
    }
    #[tokio::test]
    async fn non_https_or_local_asset_urls_fail_before_fetching() {
        let stable = Stable::new("fictional", crate::models::Environment::Sandbox).unwrap();
        for url in [
            "http://example.com/a",
            "https://user:password@example.com/a",
            "https://example.com:8443/a",
            "https://127.0.0.1/a",
            "https://[::1]/a",
        ] {
            assert!(stable.asset(url, 1024).await.is_err());
        }
    }
}
