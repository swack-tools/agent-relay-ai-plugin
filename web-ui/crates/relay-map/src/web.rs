//! HTTP + WebSocket server for the front end.

use crate::broadcast;
use crate::transcript;
use crate::hub::Shared;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use relay_map_core::{ClientMsg, ServerMsg};
use std::path::PathBuf;
use tower_http::services::ServeDir;

#[derive(Clone)]
pub struct AppState {
    pub hub: Shared,
    pub relay_py: Option<PathBuf>,
}

pub fn router(state: AppState, web_dir: PathBuf) -> Router {
    Router::new()
        .route("/ws", get(ws))
        .route("/api/snapshot", get(snapshot))
        .route("/api/transcript", get(transcript))
        .route("/api/health", get(|| async { "ok" }))
        .fallback_service(ServeDir::new(web_dir).append_index_html_on_directories(true))
        .with_state(state)
}

async fn snapshot(State(st): State<AppState>) -> impl IntoResponse {
    let snap = st.hub.lock().unwrap().snapshot();
    Json(snap)
}

#[derive(serde::Deserialize)]
struct TranscriptQuery {
    addr: String,
    /// `1` downloads the raw log instead of the readable view.
    #[serde(default)]
    raw: Option<String>,
}

/// An agent's session log: readable JSON by default, the raw JSONL with `raw=1`.
async fn transcript(State(st): State<AppState>, Query(q): Query<TranscriptQuery>) -> Response {
    let plan = match transcript::plan(&st.hub, &q.addr) {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };
    let raw = q.raw.as_deref().is_some_and(|r| r == "1" || r == "true");
    let max = if raw { transcript::RAW_BYTES } else { transcript::VIEW_BYTES };
    match transcript::fetch(&plan, max).await {
        Ok((bytes, len, source, notes)) if raw => {
            let name = format!("{}-{}.jsonl", plan.agent.kind.map_or("agent", |k| k.as_str()), plan.agent.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>());
            let _ = (len, notes);
            (
                [
                    (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8".to_string()),
                    (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
                    (header::HeaderName::from_static("x-relay-map-source"), source.to_string()),
                ],
                bytes,
            )
                .into_response()
        }
        Ok((bytes, len, source, notes)) => {
            let v = tokio::task::spawn_blocking(move || transcript::view(&plan.agent, &bytes, len, source, notes)).await;
            match v {
                Ok(v) => Json(v).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    }
}

async fn ws(upgrade: WebSocketUpgrade, State(st): State<AppState>) -> impl IntoResponse {
    upgrade.max_message_size(1 << 20).on_upgrade(move |sock| client(sock, st))
}

async fn client(sock: WebSocket, st: AppState) {
    let (mut tx, mut rx) = sock.split();
    // Subscribe before taking the snapshot so nothing falls between them.
    let (snap, mut events) = {
        let h = st.hub.lock().unwrap();
        (h.snapshot(), h.tx.subscribe())
    };
    let Ok(text) = serde_json::to_string(&snap) else { return };
    if tx.send(Message::Text(text)).await.is_err() {
        return;
    }
    let forward = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(m) => {
                    let Ok(text) = serde_json::to_string(&m) else { continue };
                    if tx.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    while let Some(Ok(msg)) = rx.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(cm) = serde_json::from_str::<ClientMsg>(&text) else { continue };
        match cm {
            ClientMsg::Broadcast { request_id, scope, subject, body, dry_run, include_stale } => {
                let (targets, skipped) = broadcast::plan(&st.hub, &scope, include_stale);
                let _ = st.hub.lock().unwrap().tx.send(ServerMsg::BroadcastPlan {
                    request_id: request_id.clone(),
                    targets: targets.clone(),
                    skipped,
                });
                if !dry_run && !targets.is_empty() && !body.trim().is_empty() {
                    let (hub, relay) = (st.hub.clone(), st.relay_py.clone());
                    tokio::spawn(broadcast::send_all(hub, relay, request_id, scope, subject, body, targets));
                }
            }
        }
    }
    forward.abort();
}
