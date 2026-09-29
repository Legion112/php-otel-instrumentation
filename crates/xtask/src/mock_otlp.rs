//! Minimal OTLP/HTTP JSON receiver for PHP integration tests.

use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server, StatusCode};

pub struct MockOtlp {
    payloads: Arc<Mutex<Vec<Value>>>,
    join: Option<JoinHandle<()>>,
    stop: Arc<Mutex<bool>>,
}

impl MockOtlp {
    pub fn start(port: u16, out_file: Option<PathBuf>) -> Result<Self, String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let server = Server::http(addr).map_err(|e| format!("bind {addr}: {e}"))?;
        let payloads = Arc::new(Mutex::new(Vec::<Value>::new()));
        if let Some(ref path) = out_file {
            std::fs::write(path, "[]").map_err(|e| format!("write {}: {e}", path.display()))?;
        }
        let stop = Arc::new(Mutex::new(false));
        let payloads_t = Arc::clone(&payloads);
        let stop_t = Arc::clone(&stop);
        let out_t = out_file;
        println!("otlp-mock listening on :{port}");
        let join = thread::spawn(move || {
            // Unblock recv periodically so we can shut down.
            server.unblock();
            loop {
                if *stop_t.lock().unwrap() {
                    break;
                }
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(mut req)) => {
                        if *req.method() == Method::Post {
                            let mut body = Vec::new();
                            if std::io::Read::read_to_end(&mut req.as_reader(), &mut body).is_ok()
                            {
                                let payload = match serde_json::from_slice::<Value>(&body) {
                                    Ok(v) => v,
                                    Err(_) => json!({
                                        "raw": String::from_utf8_lossy(&body),
                                    }),
                                };
                                let mut store = payloads_t.lock().unwrap();
                                store.push(payload);
                                if let Some(ref path) = out_t {
                                    if let Ok(text) = serde_json::to_string_pretty(&*store) {
                                        let _ = std::fs::write(path, text);
                                    }
                                }
                            }
                        }
                        let header = Header::from_bytes("Content-Type", "application/json")
                            .expect("header");
                        let _ = req.respond(Response::new(
                            StatusCode(200),
                            vec![header],
                            Cursor::new(b"{}".to_vec()),
                            Some(2),
                            None,
                        ));
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            payloads,
            join: Some(join),
            stop,
        })
    }

    pub fn payloads(&self) -> Vec<Value> {
        self.payloads.lock().unwrap().clone()
    }

    pub fn shutdown(mut self) {
        *self.stop.lock().unwrap() = true;
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
