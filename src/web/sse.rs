#![allow(dead_code)]

use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use super::WebState;

/// Query parameters for the SSE log stream endpoint.
#[derive(Deserialize)]
pub struct LogParams {
    /// Optional server name to filter logs. If absent, all servers are included.
    pub server: Option<String>,
}

/// GET /logs/stream -- Server-Sent Events stream of live log lines.
///
/// Bridges `LogAggregator.subscribe()` (broadcast::Receiver) to an SSE response
/// via `BroadcastStream`. Lagged events are silently dropped -- the HTMX SSE
/// extension reconnects automatically.
///
/// Per D-10: Uses SSE event name "log" for `sse-swap="log"` in the template.
/// Per D-11: Filters by server name when `?server=name` query param is present.
pub async fn log_stream_handler(
    State(state): State<Arc<WebState>>,
    Query(params): Query<LogParams>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.log_agg.subscribe();
    let filter_server = params.server;

    let stream = BroadcastStream::new(rx).filter_map(
        move |result: Result<crate::logs::LogLine, BroadcastStreamRecvError>| {
            let filter = filter_server.clone();
            async move {
                // Drop lagged errors silently -- HTMX SSE extension auto-reconnects.
                let line = result.ok()?;

                // Apply server filter when set.
                if let Some(ref name) = filter {
                    if &line.server != name {
                        return None;
                    }
                }

                let ts = crate::logs::format_system_time(line.timestamp);
                let data = format!(
                    "<div class=\"log-line\"><span style=\"color: #569cd6;\">[{}]</span> <span style=\"color: #808080;\">{}</span> {}</div>",
                    line.server, ts, line.message
                );

                Some(Ok(Event::default().event("log").data(data)))
            }
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
}
