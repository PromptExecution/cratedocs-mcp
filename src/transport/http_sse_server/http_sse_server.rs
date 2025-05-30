use axum::{
    body::Body,
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::get,
    Router,
};
use futures::{Stream, StreamExt, TryStreamExt};
use mcp_server::{ByteTransport, Server};
use std::collections::HashMap;
use tokio_util::codec::FramedRead;

#[cfg(test)]
// Tests in ../tests.rs

use anyhow::Result;
use mcp_server::router::RouterService;
use crate::{transport::jsonrpc_frame_codec::JsonRpcFrameCodec, tools::DocRouter};
use std::sync::Arc;
use tokio::{
    io::{self, AsyncWriteExt},
    sync::Mutex,
};

type C2SWriter = Arc<Mutex<io::WriteHalf<io::SimplexStream>>>;
type SessionId = Arc<str>;

#[derive(Clone, Default)]
pub struct App {
    pub txs: Arc<tokio::sync::RwLock<HashMap<SessionId, C2SWriter>>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            txs: Default::default(),
        }
    }
    pub fn router(&self) -> Router {
        Router::new()
            .route("/sse", get(sse_handler).post(post_event_handler))
            .with_state(self.clone())
    }
}

fn session_id() -> SessionId {
    let id = format!("{:016x}", rand::random::<u128>());
    Arc::from(id)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostEventQuery {
    #[serde(default)] // Use None if session_id is not present in query
    pub session_id: Option<String>,
}

async fn post_event_handler(
    State(app): State<App>,
    Query(query_params): Query<PostEventQuery>,
    body: Body,
) -> Result<StatusCode, StatusCode> {
    tracing::debug!(?query_params, "Received POST request");
    const BODY_BYTES_LIMIT: usize = 1 << 22;
    const BUFFER_SIZE: usize = 1 << 12; // For new sessions

    let (session_id_arc, c2s_writer_for_body): (SessionId, C2SWriter) =
        match query_params.session_id {
            Some(id_str) => {
                tracing::debug!(session_id = %id_str, "sessionId provided in query");
                // Convert String to Arc<str> for map lookup
                let session_arc: SessionId = Arc::from(id_str.as_str());
                let rg = app.txs.read().await;
                match rg.get(&session_arc) {
                    Some(writer) => {
                        tracing::debug!(session_id = %session_arc, "Found existing session writer");
                        (session_arc, writer.clone())
                    }
                    None => {
                        tracing::warn!(session_id = %session_arc, "sessionId provided but not found in active sessions");
                        return Err(StatusCode::NOT_FOUND);
                    }
                }
            }
            None => {
                tracing::info!("sessionId not provided, creating new session for POST request");
                let new_session_id_arc = session_id(); // fn session_id() -> Arc<str>
                tracing::info!(new_session_id = %new_session_id_arc, "Generated new session ID");

                let (c2s_read, c2s_write_half) = tokio::io::simplex(BUFFER_SIZE);
                // s2c_read/write are also needed for the ByteTransport and Server::run
                // _s2c_read is not directly used by this POST handler but needed for the spawned server task.
                let (_s2c_read, s2c_write_half) = tokio::io::simplex(BUFFER_SIZE);

                let new_c2s_writer_for_map = Arc::new(Mutex::new(c2s_write_half));
                app.txs
                    .write()
                    .await
                    .insert(new_session_id_arc.clone(), new_c2s_writer_for_map.clone());
                tracing::info!(session_id = %new_session_id_arc, "Inserted new session writer into app.txs");

                // Spawn the server task for the new session
                let app_clone = app.clone();
                let task_session_id = new_session_id_arc.clone();
                tokio::spawn(async move {
                    let router = RouterService(DocRouter::new());
                    let server = Server::new(router);
                    let bytes_transport = ByteTransport::new(c2s_read, s2c_write_half);
                    tracing::info!(session_id = %task_session_id, "Spawning server task for new POST session");
                    let _result = server
                        .run(bytes_transport)
                        .await
                        .inspect_err(|e| {
                            tracing::error!(?e, session_id = %task_session_id, "Server run error for new POST session")
                        });
                    app_clone.txs.write().await.remove(&task_session_id);
                    tracing::info!(session_id = %task_session_id, "Cleaned up new POST session from app.txs after server task completion");
                });
                (new_session_id_arc, new_c2s_writer_for_map)
            }
        };

    // Process the request body using c2s_writer_for_body
    let mut write_stream_locked = c2s_writer_for_body.lock().await;
    let mut body_data_stream = body.into_data_stream();

    if let (_, Some(size_hint)) = body_data_stream.size_hint() {
        if size_hint > BODY_BYTES_LIMIT {
            tracing::warn!(%session_id_arc, body_size_hint = size_hint, limit = BODY_BYTES_LIMIT, "Payload too large based on hint");
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
    }

    let mut actual_size = 0;
    while let Some(chunk_result) = body_data_stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(%session_id_arc, ?e, "Error reading chunk from body stream");
                return Err(StatusCode::BAD_REQUEST);
            }
        };
        actual_size += chunk.len();
        if actual_size > BODY_BYTES_LIMIT {
            tracing::warn!(%session_id_arc, actual_body_size = actual_size, limit = BODY_BYTES_LIMIT, "Payload too large during streaming");
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        if let Err(e) = write_stream_locked.write_all(&chunk).await {
            tracing::error!(%session_id_arc, ?e, "Error writing chunk to session stream");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    if let Err(e) = write_stream_locked.write_u8(b'\n').await {
        tracing::error!(%session_id_arc, ?e, "Error writing newline to session stream");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    tracing::info!(%session_id_arc, "Successfully processed POST request body");
    Ok(StatusCode::ACCEPTED)
}

async fn sse_handler(State(app): State<App>) -> Sse<impl Stream<Item = Result<Event, io::Error>>> {
    // it's 4KB
    const BUFFER_SIZE: usize = 1 << 12;
    let session = session_id();
    tracing::info!(%session, "sse connection");
    let (c2s_read, c2s_write) = tokio::io::simplex(BUFFER_SIZE);
    let (s2c_read, s2c_write) = tokio::io::simplex(BUFFER_SIZE);
    app.txs
        .write()
        .await
        .insert(session.clone(), Arc::new(Mutex::new(c2s_write)));
    {
        let app_clone = app.clone();
        let session = session.clone();
        tokio::spawn(async move {
            let router = RouterService(DocRouter::new());
            let server = Server::new(router);
            let bytes_transport = ByteTransport::new(c2s_read, s2c_write);
            let _result = server
                .run(bytes_transport)
                .await
                .inspect_err(|e| tracing::error!(?e, "server run error"));
            app_clone.txs.write().await.remove(&session);
        });
    }

    let stream = futures::stream::once(futures::future::ok(
        Event::default()
            .event("endpoint")
            .data(format!("?sessionId={session}")),
    ))
    .chain(
        FramedRead::new(s2c_read, JsonRpcFrameCodec)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            .and_then(move |bytes| match std::str::from_utf8(bytes.as_ref()) {
                Ok(message) => futures::future::ok(Event::default().event("message").data(message)),
                Err(e) => futures::future::err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }),
    );
    Sse::new(stream)
}