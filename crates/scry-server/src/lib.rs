//! HTTP server over the scry core engine: bearer-auth JSON API for search,
//! manifest-diff sync, and status.

pub mod api;
mod auth;
mod error;
mod routes;
mod store_actor;
pub mod tavily;

pub use store_actor::StoreHandle;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::{Router, middleware};
use scry_core::Result;
use scry_core::chat::ChatClient;
use scry_core::config::{Config, HydeMode, IndexConfig, MemoryConfig};
use scry_core::embed::{Embedder, HttpEmbedder};
use scry_core::store::Store;

const MAX_SYNC_BODY_BYTES: usize = 64 * 1024 * 1024;

pub struct AppState {
    pub store: StoreHandle,
    pub embedder: Box<dyn Embedder>,
    pub chat: Option<ChatClient>,
    pub hyde: HydeMode,
    pub auth_token: Option<String>,
    pub index_config: IndexConfig,
    pub memory_config: MemoryConfig,
    pub tavily: Option<tavily::TavilyClient>,
}

impl AppState {
    pub fn from_config(config: &Config) -> Result<Self> {
        let store = Store::open(
            &config.server.db_path,
            &config.embedding.model,
            config.embedding.dim,
        )?;
        Ok(Self::new(store, config))
    }

    pub fn new(store: Store, config: &Config) -> Self {
        Self {
            store: StoreHandle::spawn(store),
            embedder: Box::new(HttpEmbedder::new(config.embedding.clone())),
            chat: config.chat.clone().map(ChatClient::new),
            hyde: config.search.hyde,
            auth_token: config.server.auth_token.clone(),
            index_config: config.index.clone(),
            memory_config: config.memory.clone(),
            tavily: config.tavily.clone().map(tavily::TavilyClient::new),
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/v1/search", post(routes::search))
        .route("/v1/manifest", post(routes::manifest))
        .route("/v1/sync", post(routes::sync))
        .route("/v1/repos/prune", post(routes::prune))
        .route("/v1/status", get(routes::status))
        .route("/v1/memories/remember", post(routes::remember))
        .route("/v1/memories/recall", post(routes::recall))
        .route("/v1/memories/feedback", post(routes::feedback))
        .route("/v1/web/search", post(routes::web_search))
        .route("/v1/answer", post(routes::answer))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        .layer(DefaultBodyLimit::max(MAX_SYNC_BODY_BYTES));
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(api)
        .with_state(state)
}

pub async fn serve(config: Config) -> Result<()> {
    let listen = config.server.listen.clone();
    if config.server.auth_token.is_none() && !listen.starts_with("127.0.0.1") {
        tracing::warn!("listening on {listen} without an auth token");
    }
    let state = Arc::new(AppState::from_config(&config)?);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!("scry serving on {listen}");
    axum::serve(listener, router(state.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    state.store.call(|store| store.optimize()).await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("sigterm handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await.ok();
}
