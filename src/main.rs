use anyhow::{Context as _, Result};
use ha_mail::{api, config::Config, stable::Stable, store::Store, supervisor::Supervisor};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load()?;
    if let Some(path) = &config.openapi {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&api::openapi()?)?),
        )?;
        return Ok(());
    }
    let store = Store::open(&config.database, config.environment, None).await?;
    let stable = Stable::new(
        config
            .stable_key
            .as_deref()
            .context("Stable credential required")?,
        config.environment,
    )?;
    if let Some(location) = config.location_id.as_deref() {
        if let Some(binding) = store.binding().await? {
            anyhow::ensure!(
                binding.location_id == location,
                "mailbox binding does not match this deployment"
            );
        } else {
            stable.location(location).await?;
            store.bind(config.environment, location).await?;
        }
    }
    let mut context = api::Context::new(
        store.clone(),
        stable.clone(),
        config.environment,
        config
            .api_token
            .as_deref()
            .context("owner credential required")?,
    );
    if let Some(token) = &config.integration_token {
        context = context.with_integration(token);
    }
    let server = api::start(config.bind, Arc::new(context.clone()))?;
    let ingress = config
        .ingress_bind
        .map(|bind| api::start(bind, Arc::new(context.for_ingress(config.ingress_peer))))
        .transpose()?;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let mut worker = tokio::spawn(ha_mail::sync::worker(
        store.clone(),
        stable,
        config.poll_seconds,
        stopped.clone(),
    ));
    let discovery = if let (Some(supervisor), Some(token)) =
        (Supervisor::from_environment()?, config.integration_token)
    {
        Some(tokio::spawn(async move {
            let mut stopped = stopped;
            loop {
                tokio::select! {_=stopped.changed()=>break,_=supervisor.announce(&token)=>{}}
                tokio::select! {_=stopped.changed()=>break,_=tokio::time::sleep(std::time::Duration::from_secs(60))=>{}}
            }
        }))
    } else {
        None
    };
    let (shutdown_result, worker_exited) = tokio::select! {
        result = shutdown() => (result, false),
        _ = &mut worker => (Err(anyhow::anyhow!("mail sync worker stopped")), true),
    };
    let api_result = server.close().await;
    let ingress_result = if let Some(ingress) = ingress {
        ingress.close().await
    } else {
        Ok(())
    };
    let _ = stop.send(true);
    if !worker_exited {
        worker.await??;
    }
    if let Some(discovery) = discovery {
        discovery.await?;
    }
    let database_result = store.close().await;
    api_result.map_err(|_| anyhow::anyhow!("close API listener"))?;
    ingress_result.map_err(|_| anyhow::anyhow!("close ingress listener"))?;
    database_result?;
    shutdown_result?;
    Ok(())
}
async fn shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {result=tokio::signal::ctrl_c()=>result?,_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
