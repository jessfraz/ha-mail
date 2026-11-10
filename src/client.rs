use reqwest::{
    Client, Method, RequestBuilder, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Transport {
    Native,
    HomeAssistant,
}
impl Transport {
    fn url_env(self) -> &'static str {
        match self {
            Self::Native => "MAIL_API_URL",
            Self::HomeAssistant => "HOME_ASSISTANT_URL",
        }
    }
    fn token_env(self) -> &'static str {
        match self {
            Self::Native => "MAIL_API_TOKEN",
            Self::HomeAssistant => "HOME_ASSISTANT_TOKEN",
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Failure {
    pub status: Option<u16>,
    pub error_code: &'static str,
}
impl Failure {
    pub fn local(code: &'static str) -> Self {
        Self {
            status: None,
            error_code: code,
        }
    }
    fn http(status: u16, code: &'static str) -> Self {
        Self {
            status: Some(status),
            error_code: code,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct Output<T> {
    pub status: u16,
    pub etag: Option<String>,
    pub body: T,
}
#[derive(Serialize)]
pub struct SavedDocument {
    pub path: PathBuf,
    pub bytes: usize,
    pub sha256: String,
}
pub struct Api {
    client: Client,
    origin: Url,
    authorization: Option<HeaderValue>,
    transport: Transport,
}
impl Api {
    pub fn new(
        transport: Transport,
        url: Option<String>,
        token_file: Option<&Path>,
        timeout: u64,
        anonymous: bool,
    ) -> Result<Self, Failure> {
        let configured = url
            .or_else(|| std::env::var(transport.url_env()).ok())
            .ok_or_else(|| Failure::local("url_required"))?;
        let origin = Url::parse(&configured).map_err(|_| Failure::local("invalid_url"))?;
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(Failure::local("invalid_url"));
        }
        let authorization = if anonymous && transport == Transport::Native {
            None
        } else {
            let token = if let Some(path) = token_file {
                use std::os::unix::fs::PermissionsExt;
                let metadata = std::fs::symlink_metadata(path)
                    .map_err(|_| Failure::local("credential_read_failed"))?;
                if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
                    return Err(Failure::local("credential_permissions"));
                }
                let mut bytes = Vec::new();
                std::fs::File::open(path)
                    .map_err(|_| Failure::local("credential_read_failed"))?
                    .take(4097)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Failure::local("credential_read_failed"))?;
                if bytes.len() > 4096 {
                    return Err(Failure::local("invalid_credential"));
                }
                String::from_utf8(bytes)
                    .map_err(|_| Failure::local("invalid_credential"))?
                    .trim_end_matches(['\r', '\n'])
                    .to_owned()
            } else {
                std::env::var(transport.token_env())
                    .map_err(|_| Failure::local("credential_required"))?
            };
            if token.is_empty()
                || token.len() > 4096
                || !token.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err(Failure::local("invalid_credential"));
            }
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| Failure::local("invalid_credential"))?;
            value.set_sensitive(true);
            Some(value)
        };
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| Failure::local("client_setup_failed"))?;
        Ok(Self {
            client,
            origin,
            authorization,
            transport,
        })
    }
    pub fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, Failure> {
        let path = match self.transport {
            Transport::Native => path.to_owned(),
            Transport::HomeAssistant if path == "/health" => "/api/ha_mail/v1/health".into(),
            Transport::HomeAssistant => format!("/api/ha_mail{path}"),
        };
        let url = self
            .origin
            .join(&path)
            .map_err(|_| Failure::local("invalid_path"))?;
        let mut request = self.client.request(method, url);
        if let Some(auth) = &self.authorization {
            request = request.header(AUTHORIZATION, auth.clone());
        }
        Ok(request)
    }
}
pub async fn send<T: DeserializeOwned + Serialize>(
    request: RequestBuilder,
) -> Result<Output<Option<T>>, Failure> {
    let response = request
        .send()
        .await
        .map_err(|_| Failure::local("request_failed"))?;
    let status = response.status().as_u16();
    let etag = response
        .headers()
        .get("etag")
        .and_then(|s| s.to_str().ok())
        .map(str::to_owned);
    let bytes = read(response, 8 * 1024 * 1024).await?;
    let body = if status == 204 {
        None
    } else {
        Some(
            serde_json::from_slice(&bytes)
                .map_err(|_| Failure::http(status, "invalid_json_response"))?,
        )
    };
    Ok(Output { status, etag, body })
}
async fn read(mut response: reqwest::Response, maximum: usize) -> Result<Vec<u8>, Failure> {
    let status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(Failure::http(
            status,
            if (300..400).contains(&status) {
                "redirect_refused"
            } else {
                "http_error"
            },
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Failure::http(status, "response_read_failed"))?
    {
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            return Err(Failure::http(status, "response_too_large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn download(
    request: RequestBuilder,
    path: &Path,
) -> Result<Output<SavedDocument>, Failure> {
    use std::os::unix::fs::OpenOptionsExt;
    let response = request
        .send()
        .await
        .map_err(|_| Failure::local("request_failed"))?;
    let status = response.status().as_u16();
    let bytes = read(response, crate::stable::MAX_DOCUMENT).await?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(Failure::http(status, "invalid_document_type"));
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| Failure::local("output_create_failed"))?;
        file.write_all(&bytes)
            .map_err(|_| Failure::local("output_write_failed"))?;
        file.sync_all()
            .map_err(|_| Failure::local("output_write_failed"))?;
        std::fs::hard_link(&temporary, path).map_err(|e| {
            Failure::local(if e.kind() == std::io::ErrorKind::AlreadyExists {
                "output_exists"
            } else {
                "output_publish_failed"
            })
        })?;
        let directory = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| Failure::local("output_publish_failed"))?;
        Ok(Output {
            status,
            etag: None,
            body: SavedDocument {
                path: path.to_path_buf(),
                bytes: bytes.len(),
                sha256: Sha256::digest(&bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            },
        })
    })();
    match std::fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Failure::local("temporary_cleanup_failed")),
    }
    result
}
pub fn write_json(writer: impl Write, value: &impl Serialize) -> Result<(), Failure> {
    let mut writer = writer;
    serde_json::to_writer(&mut writer, value).map_err(|_| Failure::local("output_failed"))?;
    writer
        .write_all(b"\n")
        .map_err(|_| Failure::local("output_failed"))
}
