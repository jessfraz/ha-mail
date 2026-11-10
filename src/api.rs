use crate::{
    installation::{Installation, InstallationReport, PresenceReport},
    models::{
        Binding, Environment, Event, ListQuery, MailItem, Ocr, Page, Summary, SyncOperation,
        SyncStatus,
    },
    stable::{Connection, Location, ProviderError, Stable},
    store::Store,
};
use anyhow::{Context as _, Result};
use dropshot::{
    ApiDescription, Body, ClientErrorStatusCode as Status, ConfigDropshot, HttpError,
    HttpResponseHeaders, HttpResponseOk, HttpResponseUpdatedNoContent, HttpServer,
    HttpServerStarter, Path, Query, RequestContext, TypedBody, endpoint,
};
use http::Response;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct Context {
    pub store: Store,
    pub stable: Stable,
    pub environment: Environment,
    owner: [u8; 32],
    integration: Option<[u8; 32]>,
    ingress_peer: Option<IpAddr>,
    installation: Arc<Installation>,
}
impl Context {
    pub fn new(store: Store, stable: Stable, environment: Environment, owner: &str) -> Self {
        Self {
            store,
            stable,
            environment,
            owner: Sha256::digest(owner).into(),
            integration: None,
            ingress_peer: None,
            installation: Arc::new(Installation::default()),
        }
    }
    pub fn with_integration(mut self, token: &str) -> Self {
        self.integration = Some(Sha256::digest(token).into());
        self
    }
    pub fn for_ingress(mut self, peer: IpAddr) -> Self {
        self.ingress_peer = Some(peer);
        self
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Integration,
}
struct Principal {
    role: Role,
    admin: bool,
}
fn header<'a>(request: &'a RequestContext<Arc<Context>>, name: &str) -> Option<&'a str> {
    request
        .request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
}
fn fail(status: Status, code: &str) -> HttpError {
    HttpError::for_client_error(Some(code.into()), status, code.into())
}
fn ingress_access(request: &RequestContext<Arc<Context>>) -> Result<(), HttpError> {
    if request
        .context()
        .ingress_peer
        .is_some_and(|p| p != request.request.remote_addr().ip())
    {
        return Err(fail(Status::FORBIDDEN, "ingress_peer_required"));
    }
    Ok(())
}
fn authenticate(request: &RequestContext<Arc<Context>>) -> Result<Principal, HttpError> {
    let context = request.context();
    if context.ingress_peer.is_some() {
        ingress_access(request)?;
        if header(request, "x-remote-user-id").is_none_or(|s| s.is_empty() || s.len() > 200) {
            return Err(fail(Status::UNAUTHORIZED, "ha_user_required"));
        }
        if request.request.method() != http::Method::GET
            && header(request, "x-mail-request") != Some("1")
        {
            return Err(fail(Status::FORBIDDEN, "browser_request_required"));
        }
        // Supervisor enforces panel_admin on this separate, peer-bound listener.
        return Ok(Principal {
            role: Role::Owner,
            admin: true,
        });
    }
    let token = header(request, "authorization")
        .and_then(|s| s.strip_prefix("Bearer "))
        .filter(|s| s.len() <= 256)
        .ok_or_else(|| fail(Status::UNAUTHORIZED, "credential_required"))?;
    let digest: [u8; 32] = Sha256::digest(token).into();
    if bool::from(digest.ct_eq(&context.owner)) {
        return Ok(Principal {
            role: Role::Owner,
            admin: true,
        });
    }
    if context
        .integration
        .as_ref()
        .is_some_and(|v| bool::from(digest.ct_eq(v)))
    {
        return Ok(Principal {
            role: Role::Integration,
            admin: header(request, "x-mail-user-is-admin") == Some("1")
                && header(request, "x-mail-user-id")
                    .is_some_and(|s| !s.is_empty() && s.len() <= 200),
        });
    }
    Err(fail(Status::UNAUTHORIZED, "invalid_credential"))
}
fn administrator(request: &RequestContext<Arc<Context>>) -> Result<Principal, HttpError> {
    let p = authenticate(request)?;
    if !p.admin {
        return Err(fail(Status::FORBIDDEN, "administrator_required"));
    }
    Ok(p)
}
fn owner(request: &RequestContext<Arc<Context>>) -> Result<(), HttpError> {
    if !matches!(authenticate(request)?.role, Role::Owner) {
        return Err(fail(Status::FORBIDDEN, "owner_required"));
    }
    Ok(())
}
fn provider(error: ProviderError) -> HttpError {
    HttpError::for_unavail(None, error.code.into())
}
#[derive(Serialize, JsonSchema)]
struct PrivateHeaders {
    #[serde(rename = "cache-control")]
    cache_control: String,
}
fn private<T: Serialize + JsonSchema + Send + Sync + 'static>(
    value: T,
) -> HttpResponseHeaders<HttpResponseOk<T>, PrivateHeaders> {
    HttpResponseHeaders::new(
        HttpResponseOk(value),
        PrivateHeaders {
            cache_control: "no-store".into(),
        },
    )
}
type Private<T> = HttpResponseHeaders<HttpResponseOk<T>, PrivateHeaders>;
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub version: String,
    pub environment: Environment,
    pub binding: Option<Binding>,
    pub role: Role,
}
#[derive(Deserialize, JsonSchema)]
struct ItemPath {
    id: String,
}
#[derive(Deserialize, JsonSchema)]
struct OperationPath {
    id: uuid::Uuid,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EventQuery {
    after: Option<i64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LocationQuery {
    page_token: Option<String>,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectLocation {
    pub location_id: String,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    pub idempotency_key: String,
}

#[endpoint {method=GET,path="/health",tags=["system"]}]
async fn health(
    request: RequestContext<Arc<Context>>,
) -> Result<HttpResponseOk<String>, HttpError> {
    ingress_access(&request)?;
    request.context().store.ready().await?;
    Ok(HttpResponseOk("ok".into()))
}
#[endpoint {method=GET,path="/v1/system",tags=["system"]}]
async fn system(request: RequestContext<Arc<Context>>) -> Result<Private<System>, HttpError> {
    let principal = authenticate(&request)?;
    Ok(private(System {
        version: env!("CARGO_PKG_VERSION").into(),
        environment: request.context().environment,
        binding: request.context().store.binding().await?,
        role: principal.role,
    }))
}
#[endpoint {method=GET,path="/v1/summary",tags=["mail"]}]
async fn summary(request: RequestContext<Arc<Context>>) -> Result<Private<Summary>, HttpError> {
    authenticate(&request)?;
    Ok(private(request.context().store.summary().await?))
}
#[endpoint {method=GET,path="/v1/items",tags=["mail"]}]
async fn items_list(
    request: RequestContext<Arc<Context>>,
    query: Query<ListQuery>,
) -> Result<Private<Page<MailItem>>, HttpError> {
    administrator(&request)?;
    Ok(private(
        request.context().store.list(&query.into_inner()).await?,
    ))
}
#[endpoint {method=GET,path="/v1/items/{id}",tags=["mail"]}]
async fn item_get(
    request: RequestContext<Arc<Context>>,
    path: Path<ItemPath>,
) -> Result<Private<MailItem>, HttpError> {
    administrator(&request)?;
    Ok(private(
        request.context().store.item(&path.into_inner().id).await?,
    ))
}
#[endpoint {method=GET,path="/v1/items/{id}/events",tags=["mail"]}]
async fn item_events(
    request: RequestContext<Arc<Context>>,
    path: Path<ItemPath>,
    query: Query<EventQuery>,
) -> Result<Private<Page<Event>>, HttpError> {
    administrator(&request)?;
    let id = path.into_inner().id;
    request.context().store.item(&id).await?;
    Ok(private(
        request
            .context()
            .store
            .events(query.into_inner().after.unwrap_or(0), Some(&id))
            .await?,
    ))
}
#[endpoint {method=GET,path="/v1/events",tags=["mail"]}]
async fn events(
    request: RequestContext<Arc<Context>>,
    query: Query<EventQuery>,
) -> Result<Private<Page<Event>>, HttpError> {
    authenticate(&request)?;
    Ok(private(
        request
            .context()
            .store
            .events(query.into_inner().after.unwrap_or(0), None)
            .await?,
    ))
}
#[endpoint {method=GET,path="/v1/events/head",tags=["mail"]}]
async fn event_head(
    request: RequestContext<Arc<Context>>,
) -> Result<Private<crate::models::EventHead>, HttpError> {
    authenticate(&request)?;
    Ok(private(request.context().store.event_head().await?))
}
async fn observed_item(
    context: &Context,
    id: &str,
) -> Result<crate::stable::ProviderItem, HttpError> {
    context.store.item(id).await?;
    let binding = context
        .store
        .binding()
        .await?
        .ok_or_else(|| fail(Status::CONFLICT, "location_required"))?;
    context
        .stable
        .item(id, &binding.location_id)
        .await
        .map_err(provider)
}
#[endpoint {method=GET,path="/v1/items/{id}/pdf",tags=["documents"]}]
async fn pdf(
    request: RequestContext<Arc<Context>>,
    path: Path<ItemPath>,
) -> Result<Response<Body>, HttpError> {
    administrator(&request)?;
    let item = observed_item(request.context(), &path.into_inner().id).await?;
    let document = request
        .context()
        .stable
        .document(&item, true)
        .await
        .map_err(provider)?;
    document_response(document.bytes, document.content_type)
}
#[endpoint {method=GET,path="/v1/items/{id}/envelope",tags=["documents"]}]
async fn envelope(
    request: RequestContext<Arc<Context>>,
    path: Path<ItemPath>,
) -> Result<Response<Body>, HttpError> {
    administrator(&request)?;
    let item = observed_item(request.context(), &path.into_inner().id).await?;
    let document = request
        .context()
        .stable
        .document(&item, false)
        .await
        .map_err(provider)?;
    document_response(document.bytes, document.content_type)
}
#[endpoint {method=GET,path="/v1/items/{id}/ocr",tags=["documents"]}]
async fn ocr(
    request: RequestContext<Arc<Context>>,
    path: Path<ItemPath>,
) -> Result<Private<Ocr>, HttpError> {
    administrator(&request)?;
    let item = observed_item(request.context(), &path.into_inner().id).await?;
    Ok(private(
        request
            .context()
            .stable
            .ocr(&item)
            .await
            .map_err(provider)?,
    ))
}
#[endpoint {method=GET,path="/v1/locations",tags=["installation"]}]
async fn locations(
    request: RequestContext<Arc<Context>>,
    query: Query<LocationQuery>,
) -> Result<Private<Page<Location>>, HttpError> {
    owner(&request)?;
    let query = query.into_inner();
    if query.page_token.as_ref().is_some_and(|s| s.len() > 4096) {
        return Err(fail(Status::BAD_REQUEST, "invalid_page_token"));
    }
    let connection: Connection<Location> = request
        .context()
        .stable
        .locations(query.page_token.as_deref())
        .await
        .map_err(provider)?;
    Ok(private(Page {
        items: connection.edges.into_iter().map(|e| e.node).collect(),
        next_page_token: if connection.page_info.has_next_page {
            connection.page_info.end_cursor
        } else {
            None
        },
    }))
}

#[endpoint {method=GET,path="/v1/installation/location",tags=["installation"]}]
async fn bound_location(
    request: RequestContext<Arc<Context>>,
) -> Result<Private<Location>, HttpError> {
    owner(&request)?;
    let binding = request
        .context()
        .store
        .binding()
        .await?
        .ok_or(crate::error::Error::NotFound)?;
    Ok(private(
        request
            .context()
            .stable
            .location(&binding.location_id)
            .await
            .map_err(provider)?,
    ))
}
#[endpoint {method=POST,path="/v1/installation/location",tags=["installation"]}]
async fn select_location(
    request: RequestContext<Arc<Context>>,
    body: TypedBody<SelectLocation>,
) -> Result<Private<Binding>, HttpError> {
    owner(&request)?;
    let location = body.into_inner().location_id;
    request
        .context()
        .stable
        .location(&location)
        .await
        .map_err(provider)?;
    Ok(private(
        request
            .context()
            .store
            .bind(request.context().environment, &location)
            .await?,
    ))
}
#[endpoint {method=GET,path="/v1/sync",tags=["sync"]}]
async fn sync_status(
    request: RequestContext<Arc<Context>>,
) -> Result<Private<SyncStatus>, HttpError> {
    authenticate(&request)?;
    Ok(private(request.context().store.sync_status().await?))
}
#[endpoint {method=POST,path="/v1/sync",tags=["sync"]}]
async fn sync_request(
    request: RequestContext<Arc<Context>>,
    body: TypedBody<SyncRequest>,
) -> Result<Private<SyncOperation>, HttpError> {
    administrator(&request)?;
    if request.context().store.binding().await?.is_none() {
        return Err(fail(Status::CONFLICT, "location_required"));
    }
    Ok(private(
        request
            .context()
            .store
            .enqueue(&body.into_inner().idempotency_key)
            .await?,
    ))
}
#[endpoint {method=GET,path="/v1/operations/{id}",tags=["sync"]}]
async fn operation(
    request: RequestContext<Arc<Context>>,
    path: Path<OperationPath>,
) -> Result<Private<SyncOperation>, HttpError> {
    administrator(&request)?;
    Ok(private(
        request
            .context()
            .store
            .operation(path.into_inner().id)
            .await?,
    ))
}
#[endpoint {method=GET,path="/v1/installation",tags=["installation"]}]
async fn installation(
    request: RequestContext<Arc<Context>>,
) -> Result<Private<InstallationReport>, HttpError> {
    owner(&request)?;
    Ok(private(request.context().installation.report().await))
}
#[endpoint {method=POST,path="/v1/installation/check",tags=["installation"]}]
async fn installation_check(
    request: RequestContext<Arc<Context>>,
) -> Result<Private<InstallationReport>, HttpError> {
    owner(&request)?;
    Ok(private(request.context().installation.report().await))
}
#[endpoint {method=POST,path="/v1/integration/presence",tags=["installation"]}]
async fn presence(
    request: RequestContext<Arc<Context>>,
    body: TypedBody<PresenceReport>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    let p = authenticate(&request)?;
    if !matches!(p.role, Role::Integration) || request.context().ingress_peer.is_some() {
        return Err(fail(Status::FORBIDDEN, "integration_required"));
    }
    request
        .context()
        .installation
        .record(body.into_inner())
        .await?;
    Ok(HttpResponseUpdatedNoContent())
}

fn document_response(
    bytes: Vec<u8>,
    content_type: &'static str,
) -> Result<Response<Body>, HttpError> {
    Response::builder()
        .header("content-type", content_type)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "no-referrer")
        .header("content-security-policy", "default-src 'none'; sandbox")
        .body(Body::from(bytes))
        .map_err(|_| HttpError::for_internal_error("response unavailable".into()))
}
fn static_response(
    content: String,
    content_type: &'static str,
) -> Result<Response<Body>, HttpError> {
    Response::builder().header("content-type",content_type).header("cache-control","no-store").header("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'").header("x-content-type-options","nosniff").header("referrer-policy","no-referrer").body(Body::from(content)).map_err(|_|HttpError::for_internal_error("response unavailable".into()))
}
#[endpoint {method=GET,path="/",tags=["web"]}]
async fn index(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    let base = if request.context().ingress_peer.is_some() {
        header(&request, "x-ingress-path").unwrap_or("")
    } else {
        ""
    };
    if !base.is_empty()
        && (!base.starts_with('/')
            || base.starts_with("//")
            || base.len() > 2048
            || !base
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-')))
    {
        return Err(fail(Status::BAD_REQUEST, "invalid_ingress_path"));
    }
    static_response(
        include_str!("../web/index.html")
            .replace("__MAIL_BASE__", &format!("{}/", base.trim_end_matches('/'))),
        "text/html; charset=utf-8",
    )
}
#[endpoint {method=GET,path="/app.js",tags=["web"]}]
async fn script(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(
        include_str!("../web/app.js").into(),
        "text/javascript; charset=utf-8",
    )
}
#[endpoint {method=GET,path="/app-kit.js",tags=["web"]}]
async fn kit_script(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(ha_app_ui::SCRIPT.into(), "text/javascript; charset=utf-8")
}
#[endpoint {method=GET,path="/app-kit.css",tags=["web"]}]
async fn kit_style(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(ha_app_ui::STYLESHEET.into(), "text/css; charset=utf-8")
}
#[endpoint {method=GET,path="/shell.js",tags=["web"]}]
async fn shell_script(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(
        include_str!("../web/shell.js").into(),
        "text/javascript; charset=utf-8",
    )
}
#[endpoint {method=GET,path="/style.css",tags=["web"]}]
async fn style(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(
        include_str!("../web/style.css").into(),
        "text/css; charset=utf-8",
    )
}
#[endpoint {method=GET,path="/brand.svg",tags=["web"]}]
async fn brand(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(include_str!("../web/brand.svg").into(), "image/svg+xml")
}
#[endpoint {method=GET,path="/openapi.json",tags=["system"]}]
async fn schema(request: RequestContext<Arc<Context>>) -> Result<Response<Body>, HttpError> {
    ingress_access(&request)?;
    static_response(
        serde_json::to_string(
            &openapi().map_err(|_| HttpError::for_internal_error("schema unavailable".into()))?,
        )
        .map_err(|_| HttpError::for_internal_error("schema unavailable".into()))?,
        "application/json",
    )
}
pub fn description() -> Result<ApiDescription<Arc<Context>>> {
    let mut api = ApiDescription::new();
    let endpoints: [dropshot::ApiEndpoint<Arc<Context>>; 28] = [
        health.into(),
        system.into(),
        summary.into(),
        items_list.into(),
        item_get.into(),
        item_events.into(),
        events.into(),
        event_head.into(),
        pdf.into(),
        envelope.into(),
        ocr.into(),
        locations.into(),
        bound_location.into(),
        select_location.into(),
        sync_status.into(),
        sync_request.into(),
        operation.into(),
        installation.into(),
        installation_check.into(),
        presence.into(),
        index.into(),
        script.into(),
        kit_script.into(),
        kit_style.into(),
        shell_script.into(),
        style.into(),
        brand.into(),
        schema.into(),
    ];
    for endpoint in endpoints {
        api.register(endpoint).context("register API endpoint")?;
    }
    Ok(api)
}
pub fn openapi() -> Result<serde_json::Value> {
    description()?.openapi("Mail API",semver::Version::parse(env!("CARGO_PKG_VERSION"))?).description("Location-bound physical mail snapshots, documents, and GET-only synchronization. Stable credentials remain server-side. Physical mail actions are not implemented.").json().map_err(Into::into)
}
pub fn start(bind: SocketAddr, context: Arc<Context>) -> Result<HttpServer<Arc<Context>>> {
    HttpServerStarter::new(
        &ConfigDropshot {
            bind_address: bind,
            default_request_body_max_bytes: 16 * 1024,
            default_handler_task_mode: dropshot::HandlerTaskMode::Detached,
            ..Default::default()
        },
        description()?,
        context,
        &crate::logging::logger(),
    )
    .map_err(|_| anyhow::anyhow!("start HTTP listener"))
    .map(HttpServerStarter::start)
}
