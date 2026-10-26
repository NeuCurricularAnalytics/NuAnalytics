//! A stub `PostgREST` for tests: answers each request from a route table, and counts
//! the writes.
//!
//! Reads every request in full — headers, then `Content-Length` bytes of body — before
//! answering. A stub that answered after one read and closed would reset the connection
//! under a client still sending a large `PATCH`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::DbClient;

/// A stub backend. `GET`s answer with the body of the first route whose text appears in
/// the request line (else `[]`); every other method is counted in [`Self::writes`] and
/// answered with an empty body and `Content-Range: */1`.
pub struct Backend {
    /// The client pointed at it.
    pub client: DbClient,
    /// Requests other than `GET` so far.
    pub writes: Arc<AtomicUsize>,
}

impl Backend {
    /// How many writes it has answered.
    pub fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while let Ok(n) = stream.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// The extra headers and body for one request: a `GET` answered from the first route in
/// its request line (else `[]`), anything else counted as a write.
fn respond(
    request: &str,
    routes: &[(&'static str, String)],
    writes: &AtomicUsize,
) -> (&'static str, String) {
    let line = request.lines().next().unwrap_or("");
    if line.starts_with("GET ") {
        let body = routes
            .iter()
            .find(|(path, _)| line.contains(path))
            .map_or_else(|| "[]".to_string(), |(_, b)| b.clone());
        ("", body)
    } else {
        writes.fetch_add(1, Ordering::SeqCst);
        ("Content-Range: */1\r\n", String::new())
    }
}

/// Start a stub serving `routes` — `(text in the request line, JSON body)` pairs.
pub async fn routed(routes: Vec<(&'static str, String)>) -> Backend {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let writes = Arc::new(AtomicUsize::new(0));
    let counter = writes.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut stream).await;
            let (extra, body) = respond(&request, &routes, &counter);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    Backend {
        client: DbClient::new(&url, "anon", "jwt".to_string()).expect("stub client"),
        writes,
    }
}

/// A client nothing answers: reaching the network fails, so a test that gets a refusal
/// instead proves the refusal came first.
pub fn unreachable() -> DbClient {
    DbClient::new("http://127.0.0.1:9", "anon", "jwt".to_string()).expect("client")
}
