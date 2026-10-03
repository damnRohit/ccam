//! A minimal HTTP/1.1 server for tests: canned responses by request path, and a record of what was received.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

#[derive(Clone)]
pub enum Reply {
    Json(u16, String),
    /// Accepts the connection and never answers.
    Hang,
}

pub struct TestServer {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Request>>>,
}

impl TestServer {
    pub fn start(routes: Vec<(&str, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: HashMap<String, Reply> = routes.into_iter().map(|(p, r)| (p.to_string(), r)).collect();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let routes = routes.clone();
                let log = Arc::clone(&log);
                thread::spawn(move || {
                    let Some(request) = read_request(&mut stream) else { return };
                    let reply = routes.get(&request.path).cloned().unwrap_or(Reply::Json(404, "{}".into()));
                    log.lock().unwrap().push(request);
                    match reply {
                        Reply::Json(status, body) => {
                            let response = format!(
                                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes());
                        }
                        Reply::Hang => thread::sleep(Duration::from_secs(5)),
                    }
                });
            }
        });
        Self { base, requests }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn received(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let length = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request { method, path, headers, body: String::from_utf8_lossy(&body).into_owned() })
}
