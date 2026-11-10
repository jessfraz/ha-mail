use clap::{Args, Parser, Subcommand};
use ha_mail::{
    api::{SelectLocation, SyncRequest, System},
    client::{Api, Failure, Transport, download, send, write_json},
    installation::InstallationReport,
    models::{Binding, Event, ListQuery, MailItem, Ocr, Page, Summary, SyncOperation, SyncStatus},
    stable::Location,
};
use reqwest::Method;
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(name = "ha-mail", version, about = "One-request JSON client for Mail")]
struct Cli {
    #[arg(long, value_enum, default_value = "native", global = true)]
    transport: Transport,
    #[arg(long, global = true)]
    url: Option<String>,
    #[arg(long, global = true)]
    token_file: Option<PathBuf>,
    #[arg(long,default_value="30",value_parser=clap::value_parser!(u64).range(1..=300),global=true)]
    timeout: u64,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Health,
    System,
    Summary,
    Installation,
    InstallationCheck,
    #[command(subcommand)]
    Items(Items),
    #[command(subcommand)]
    Sync(Sync),
    /// Available locations. Select one explicitly; this never chooses the first.
    Locations {
        #[arg(long)]
        page_token: Option<String>,
    },
    /// Bind a fresh installation to one location. Existing bindings cannot change.
    Configure {
        #[arg(long,value_parser=parse_id)]
        location_id: String,
    },
    /// Read a previously returned sync operation.
    Operation {
        id: uuid::Uuid,
    },
    Events {
        #[arg(long)]
        after: Option<i64>,
    },
}
#[derive(Args)]
struct ItemPath {
    #[arg(value_parser=parse_id)]
    id: String,
}
fn parse_id(s: &str) -> Result<String, &'static str> {
    if ha_mail::models::valid_id(s) {
        Ok(s.into())
    } else {
        Err("expected a provider item ID")
    }
}
#[derive(Subcommand)]
enum Items {
    List {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        archived: bool,
        #[arg(long)]
        unread: bool,
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=200))]
        limit: Option<u32>,
        #[arg(long)]
        page_token: Option<String>,
    },
    Get(ItemPath),
    Scans(ItemPath),
    Summary(ItemPath),
    Ocr(ItemPath),
    Events {
        #[command(flatten)]
        item: ItemPath,
        #[arg(long)]
        after: Option<i64>,
    },
    Pdf {
        #[command(flatten)]
        item: ItemPath,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Subcommand)]
enum Sync {
    Status,
    Request {
        #[arg(long)]
        idempotency_key: String,
    },
}
impl Command {
    fn supports_ha(&self) -> bool {
        !matches!(
            self,
            Self::Locations { .. }
                | Self::Configure { .. }
                | Self::Installation
                | Self::InstallationCheck
        )
    }
}
async fn run(cli: Cli) -> Result<(), Failure> {
    if cli.transport == Transport::HomeAssistant && !cli.command.supports_ha() {
        return Err(Failure::local("unsupported_transport_operation"));
    }
    let api = Api::new(
        cli.transport,
        cli.url,
        cli.token_file.as_deref(),
        cli.timeout,
        matches!(cli.command, Command::Health),
    )?;
    macro_rules! get {
        ($ty:ty,$path:expr) => {
            write_json(
                std::io::stdout().lock(),
                &send::<$ty>(api.request(Method::GET, $path)?).await?,
            )
        };
    }
    match cli.command {
        Command::Health => get!(String, "/health"),
        Command::System => get!(System, "/v1/system"),
        Command::Summary => get!(Summary, "/v1/summary"),
        Command::Installation => get!(InstallationReport, "/v1/installation"),
        Command::InstallationCheck => write_json(
            std::io::stdout().lock(),
            &send::<InstallationReport>(api.request(Method::POST, "/v1/installation/check")?)
                .await?,
        ),
        Command::Items(Items::List {
            query,
            status,
            archived,
            unread,
            limit,
            page_token,
        }) => {
            let query = ListQuery {
                query,
                status,
                archived: Some(archived),
                unread: unread.then_some(true),
                limit,
                page_token,
            };
            write_json(
                std::io::stdout().lock(),
                &send::<Page<MailItem>>(api.request(Method::GET, "/v1/items")?.query(&query))
                    .await?,
            )
        }
        Command::Items(Items::Get(item) | Items::Scans(item) | Items::Summary(item)) => {
            get!(MailItem, &format!("/v1/items/{}", item.id))
        }
        Command::Items(Items::Ocr(item)) => get!(Ocr, &format!("/v1/items/{}/ocr", item.id)),
        Command::Items(Items::Pdf { item, output }) => write_json(
            std::io::stdout().lock(),
            &download(
                api.request(Method::GET, &format!("/v1/items/{}/pdf", item.id))?,
                &output,
            )
            .await?,
        ),
        Command::Items(Items::Events { item, after }) => write_json(
            std::io::stdout().lock(),
            &send::<Page<Event>>(
                api.request(Method::GET, &format!("/v1/items/{}/events", item.id))?
                    .query(&[("after", after.unwrap_or(0))]),
            )
            .await?,
        ),
        Command::Sync(Sync::Status) => get!(SyncStatus, "/v1/sync"),
        Command::Sync(Sync::Request { idempotency_key }) => write_json(
            std::io::stdout().lock(),
            &send::<SyncOperation>(
                api.request(Method::POST, "/v1/sync")?
                    .json(&SyncRequest { idempotency_key }),
            )
            .await?,
        ),
        Command::Locations { page_token } => write_json(
            std::io::stdout().lock(),
            &send::<Page<Location>>(
                api.request(Method::GET, "/v1/locations")?
                    .query(&[("page_token", page_token)]),
            )
            .await?,
        ),
        Command::Configure { location_id } => write_json(
            std::io::stdout().lock(),
            &send::<Binding>(
                api.request(Method::POST, "/v1/installation/location")?
                    .json(&SelectLocation { location_id }),
            )
            .await?,
        ),
        Command::Operation { id } => get!(SyncOperation, &format!("/v1/operations/{id}")),
        Command::Events { after } => write_json(
            std::io::stdout().lock(),
            &send::<Page<Event>>(
                api.request(Method::GET, "/v1/events")?
                    .query(&[("after", after.unwrap_or(0))]),
            )
            .await?,
        ),
    }
}
#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let _ = write_json(std::io::stderr().lock(), &failure);
            ExitCode::FAILURE
        }
    }
}
