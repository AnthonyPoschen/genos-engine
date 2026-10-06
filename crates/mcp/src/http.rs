//! Loopback Streamable HTTP for one MCP session.
//!
//! `POST /mcp` carries JSON-RPC. `GET /mcp` is the server-sent event stream
//! for `notifications/resources/updated`. The listener is `127.0.0.1` only.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::discover;
use crate::json::{self, Value};
use crate::live::{Host, SCENE_URI};
use crate::rpc::{self, Reply};

pub struct Server {
    url: String,
    shutdown: Arc<AtomicBool>,
    host: Host,
    listener: Option<JoinHandle<()>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Server {
    pub fn start(host: Host) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|err| err.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|err| err.to_string())?;
        let local = listener.local_addr().map_err(|err| err.to_string())?;
        if !local.ip().is_loopback() {
            return Err("mcp listener is not loopback".into());
        }
        let url = format!("http://127.0.0.1:{}/mcp", local.port());
        discover::publish(&url)?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let accept_shutdown = Arc::clone(&shutdown);
        let accept_workers = Arc::clone(&workers);
        let accept_host = host.clone();
        let listener = thread::Builder::new()
            .name("genos-mcp".into())
            .spawn(move || accept_loop(listener, accept_host, accept_shutdown, accept_workers))
            .map_err(|err| {
                discover::remove_record_if_url(&url);
                err.to_string()
            })?;
        Ok(Self {
            url,
            shutdown,
            host,
            listener: Some(listener),
            workers,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.host.wake();
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        let workers = std::mem::take(&mut *lock_workers(&self.workers));
        for worker in workers {
            let _ = worker.join();
        }
        discover::remove_record_if_url(&self.url);
    }
}

fn accept_loop(
    listener: TcpListener,
    host: Host,
    shutdown: Arc<AtomicBool>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_nodelay(true);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let host = host.clone();
                let shutdown = Arc::clone(&shutdown);
                let handle =
                    thread::Builder::new()
                        .name("genos-mcp-conn".into())
                        .spawn(move || {
                            let _ = serve_connection(stream, host, shutdown);
                        });
                if let Ok(handle) = handle {
                    lock_workers(&workers).push(handle);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

fn lock_workers(
    workers: &Mutex<Vec<JoinHandle<()>>>,
) -> std::sync::MutexGuard<'_, Vec<JoinHandle<()>>> {
    workers.lock().unwrap_or_else(|err| err.into_inner())
}

fn serve_connection(
    mut stream: TcpStream,
    host: Host,
    shutdown: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let request = read_request(&mut stream)?;
    if let Some(origin) = header(&request.headers, "origin") {
        if !origin_allowed(origin) {
            return write_empty(&mut stream, 403, "Forbidden");
        }
    }
    let path = request
        .path
        .split('?')
        .next()
        .unwrap_or(request.path.as_str());
    if path != "/mcp" {
        return write_empty(&mut stream, 404, "Not Found");
    }
    match request.method.as_str() {
        "POST" => serve_post(&mut stream, &host, &request),
        "GET" => serve_get(&mut stream, &host, &request, &shutdown),
        "DELETE" => serve_delete(&mut stream, &host, &request),
        _ => write_empty(&mut stream, 405, "Method Not Allowed"),
    }
}

fn serve_post(stream: &mut TcpStream, host: &Host, request: &Request) -> std::io::Result<()> {
    if let Some(accept) = header(&request.headers, "accept") {
        if !accept.split(',').any(|part| {
            let part = part.trim();
            part.starts_with("application/json")
                || part.starts_with("text/event-stream")
                || part.starts_with("*/*")
        }) {
            return write_empty(stream, 406, "Not Acceptable");
        }
    }
    if let Some(content_type) = header(&request.headers, "content-type") {
        if !content_type.starts_with("application/json") {
            return write_empty(stream, 415, "Unsupported Media Type");
        }
    }
    let message = match json::parse(&request.body) {
        Ok(value) => value,
        Err(err) => {
            return write_json(
                stream,
                400,
                "Bad Request",
                None,
                &error_body(&Value::Null, -32700, &err),
            );
        }
    };
    if message.as_array().is_some() {
        return write_json(
            stream,
            200,
            "OK",
            None,
            &error_body(&Value::Null, -32600, "batch is not supported"),
        );
    }
    if let Some(version) = message.get("jsonrpc").and_then(Value::as_str) {
        if version != "2.0" {
            return write_json(
                stream,
                200,
                "OK",
                None,
                &error_body(&Value::Null, -32600, "jsonrpc must be 2.0"),
            );
        }
    }
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = request_id(&message);
    if method == "initialize" {
        let Some(id) = id else {
            return write_json(
                stream,
                200,
                "OK",
                None,
                &error_body(&Value::Null, -32600, "initialize requires an id"),
            );
        };
        let params = message
            .get("params")
            .cloned()
            .unwrap_or_else(|| json::object([]));
        let session = host.open_session();
        let body = result_body(&id, rpc::initialize_result(&params));
        return write_json(stream, 200, "OK", Some(&session), &body);
    }
    let Some(session) = header(&request.headers, "mcp-session-id") else {
        return write_empty(stream, 400, "Bad Request");
    };
    if !host.session_open(session) {
        return write_empty(stream, 404, "Not Found");
    }
    if let Some(version) = header(&request.headers, "mcp-protocol-version") {
        if !rpc::known_version(version) {
            return write_empty(stream, 400, "Bad Request");
        }
    }
    let Some(id) = id else {
        return write_empty(stream, 202, "Accepted");
    };
    if method.is_empty() {
        return write_json(
            stream,
            200,
            "OK",
            Some(session),
            &error_body(&id, -32600, "missing method"),
        );
    }
    let params = message
        .get("params")
        .cloned()
        .unwrap_or_else(|| json::object([]));
    let body = match rpc::dispatch(host, session, method, &params) {
        Reply::Result(result) => result_body(&id, result),
        Reply::Error { code, message } => error_body(&id, code, &message),
    };
    write_json(stream, 200, "OK", Some(session), &body)
}

fn serve_get(
    stream: &mut TcpStream,
    host: &Host,
    request: &Request,
    shutdown: &AtomicBool,
) -> std::io::Result<()> {
    let accept = header(&request.headers, "accept").unwrap_or("");
    if !accept.split(',').any(|part| {
        let part = part.trim();
        part.starts_with("text/event-stream") || part.starts_with("*/*")
    }) {
        return write_empty(stream, 406, "Not Acceptable");
    }
    let Some(session) = header(&request.headers, "mcp-session-id") else {
        return write_empty(stream, 400, "Bad Request");
    };
    if !host.session_open(session) {
        return write_empty(stream, 404, "Not Found");
    }
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\nMcp-Session-Id: {session}\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.flush()?;
    let mut seen = host.published();
    let session = session.to_string();
    loop {
        let Some(next) = host.wait_published(&session, seen, shutdown) else {
            break;
        };
        seen = next;
        let event = resource_updated_event();
        if write_chunk(stream, &event).is_err() {
            break;
        }
    }
    let _ = write_chunk(stream, "");
    Ok(())
}

fn serve_delete(stream: &mut TcpStream, host: &Host, request: &Request) -> std::io::Result<()> {
    let Some(session) = header(&request.headers, "mcp-session-id") else {
        return write_empty(stream, 400, "Bad Request");
    };
    if !host.session_open(session) {
        return write_empty(stream, 404, "Not Found");
    }
    host.close_session(session);
    write_empty(stream, 200, "OK")
}

fn resource_updated_event() -> String {
    let body = json::encode(&json::object([
        ("jsonrpc", json::string("2.0")),
        ("method", json::string("notifications/resources/updated")),
        ("params", json::object([("uri", json::string(SCENE_URI))])),
    ]));
    format!("event: message\ndata: {body}\n\n")
}

fn result_body(id: &Value, result: Value) -> String {
    json::encode(&json::object([
        ("jsonrpc", json::string("2.0")),
        ("id", id.clone()),
        ("result", result),
    ]))
}

fn error_body(id: &Value, code: i64, message: &str) -> String {
    json::encode(&json::object([
        ("jsonrpc", json::string("2.0")),
        ("id", id.clone()),
        (
            "error",
            json::object([
                ("code", json::int(code)),
                ("message", json::string(message)),
            ]),
        ),
    ]))
}

fn request_id(message: &Value) -> Option<Value> {
    match message.get("id") {
        None | Some(Value::Null) => None,
        Some(id) => Some(id.clone()),
    }
}

fn origin_allowed(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    if rest.contains('@') || rest.contains('\\') {
        return false;
    }
    let hostport = rest.split('/').next().unwrap_or(rest);
    let host = if let Some(host) = hostport.strip_prefix('[') {
        let end = host.find(']').unwrap_or(host.len());
        &host[..end]
    } else {
        hostport.split(':').next().unwrap_or(hostport)
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let header_end = loop {
        let count = stream.read(&mut tmp)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "closed",
            ));
        }
        buf.extend_from_slice(&tmp[..count]);
        if let Some(end) = find_header_end(&buf) {
            break end;
        }
        if buf.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
    };
    let header_text = String::from_utf8_lossy(&buf[..header_end.saturating_sub(4)]).into_owned();
    let mut lines = header_text.split("\r\n");
    let Some(request_line) = lines.next() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing request line",
        ));
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    let mut body = buf[header_end..].to_vec();
    let length = header(&headers, "content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "body too large",
        ));
    }
    while body.len() < length {
        let count = stream.read(&mut tmp)?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated body",
            ));
        }
        body.extend_from_slice(&tmp[..count]);
    }
    body.truncate(length);
    let body = String::from_utf8(body)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "body is not utf-8"))?;
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn write_empty(stream: &mut TcpStream, status: u16, reason: &str) -> std::io::Result<()> {
    write_json(stream, status, reason, None, "")
}

fn write_json(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    session: Option<&str>,
    body: &str,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if let Some(session) = session {
        head.push_str("Mcp-Session-Id: ");
        head.push_str(session);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn write_chunk(stream: &mut TcpStream, data: &str) -> std::io::Result<()> {
    if data.is_empty() {
        stream.write_all(b"0\r\n\r\n")?;
    } else {
        write!(stream, "{:x}\r\n", data.len())?;
        stream.write_all(data.as_bytes())?;
        stream.write_all(b"\r\n")?;
    }
    stream.flush()
}
