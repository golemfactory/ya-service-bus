use actix_web::http::header;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use futures::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use crate::connection::{GetNodeInfo, NodeInfo};
use crate::router::{AbstractRouter, NodeEvent};

pub struct RestManager {
    pub router: Arc<dyn AbstractRouter>,
}

impl RestManager {
    pub fn new(router: Arc<dyn AbstractRouter>) -> Self {
        RestManager { router }
    }

    pub fn events(&self) -> impl Stream<Item = NodeEvent> {
        let receiver = self.router.node_events();
        BroadcastStream::new(receiver)
            .map(|it| it.unwrap_or_else(|BroadcastStreamRecvError::Lagged(n)| NodeEvent::Lag(n)))
    }
}

/// SSE endpoint for node events
pub async fn events(rm: web::Data<Arc<RestManager>>) -> impl Responder {
    let events = futures::StreamExt::map(rm.events(), |event| {
        Ok::<_, actix_web::Error>(web::Bytes::from(match event {
            NodeEvent::New(node_id) => {
                format!("event: new-node\ndata: {}\n\n", node_id)
            }
            NodeEvent::Lost(node_id) => {
                format!("event: lost-node\ndata: {}\n\n", node_id)
            }
            NodeEvent::Lag(n) => format!("event: drop\ndata: {}\n\n", n),
        }))
    })
    .boxed_local();

    HttpResponse::Ok()
        .append_header((header::CONTENT_TYPE, "text/event-stream"))
        .append_header((header::CACHE_CONTROL, "no-cache"))
        .append_header((header::CONNECTION, "keep-alive"))
        .streaming(events)
}

/// Health check and status endpoint
pub async fn health(rm: web::Data<Arc<RestManager>>) -> impl Responder {
    let response = serde_json::json!({
        "service": "ya-sb-router",
        "status": "healthy",
        "connections": rm.router.registered_instances_count(),
        "endpoints": rm.router.registered_endpoints_count(),
        "topics": rm.router.topics_count(),
    });

    HttpResponse::Ok().json(response)
}

#[derive(Serialize, Deserialize)]
struct NodesResponse {
    #[serde(flatten)]
    nodes: HashMap<String, Vec<NodeInfo>>,
    /// Number of nodes all Nodes registered on relay.
    count: usize,
}

async fn get_prefixed_nodes(
    rm: &Arc<RestManager>,
    prefix: Option<String>,
) -> Result<NodesResponse, actix_web::Error> {
    let recipients = rm.router.get_connection_recipients();
    let mut futs = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        futs.push(recipient.send(GetNodeInfo));
    }
    let results = futures::future::join_all(futs).await;

    let count = results.len();
    let prefix = prefix.map(|p| format!("0x{}", p));

    let nodes: HashMap<String, Vec<NodeInfo>> = results
        .into_iter()
        .filter_map(|res| res.ok().map(|res| res.ok()).flatten())
        .filter(|node| {
            prefix.as_ref().map_or(true, |prefix| {
                node.identities
                    .iter()
                    .any(|identity| identity.starts_with(prefix))
            })
        })
        .filter(|node| node.id.is_some())
        .fold(HashMap::new(), |mut acc, node| {
            let key = node.id.clone().unwrap();
            acc.entry(key).or_insert_with(Vec::new).push(node);
            acc
        });
    Ok(NodesResponse { count, nodes })
}

/// List all connected nodes
pub async fn nodes(rm: web::Data<Arc<RestManager>>) -> Result<impl Responder, actix_web::Error> {
    get_prefixed_nodes(&rm, None)
        .await
        .map(|response| HttpResponse::Ok().json(response))
}

/// List connected nodes filtered by prefix
pub async fn nodes_prefix(
    rm: web::Data<Arc<RestManager>>,
    path: web::Path<String>,
) -> Result<impl Responder, actix_web::Error> {
    get_prefixed_nodes(&rm, Some(path.into_inner()))
        .await
        .map(|response| HttpResponse::Ok().json(response))
}

impl RestManager {
    /// Configure web routes for this REST manager
    pub fn configure_routes(self: Arc<Self>, cfg: &mut web::ServiceConfig) {
        cfg.app_data(web::Data::new(self))
            .route("/events", web::get().to(events))
            .route("/health", web::get().to(health))
            .route("/nodes", web::get().to(nodes))
            .route("/nodes/{prefix}", web::get().to(nodes_prefix));
    }

    /// Start the web server
    pub async fn start_web_server(self: Arc<Self>, web_url: url::Url) -> anyhow::Result<()> {
        let bind_addr = web_url
            .socket_addrs(|| None)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                anyhow::anyhow!("Invalid web-addr URL '{web_url}': must include host and port")
            })?;

        log::info!("REST API server starting on: {web_url}");

        HttpServer::new(move || {
            App::new().configure(|cfg| RestManager::configure_routes(self.clone(), cfg))
        })
        .bind(bind_addr)?
        .run()
        .await?;

        Ok(())
    }
}
