#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub path: String,
    pub authorization: Option<String>,
}

/// A local HTTP server for fixture archives, so tests never touch the network.
pub struct TestServer {
    server: Arc<tiny_http::Server>,
    base: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let server = Arc::clone(&server);
            let routes = Arc::clone(&routes);
            let requests = Arc::clone(&requests);
            std::thread::spawn(move || {
                for request in server.incoming_requests() {
                    let path = request.url().to_owned();
                    let authorization = request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string());
                    requests.lock().unwrap().push(RecordedRequest {
                        path: path.clone(),
                        authorization,
                    });
                    let body = routes.lock().unwrap().get(&path).cloned();
                    let _ = match body {
                        Some(body) => request.respond(tiny_http::Response::from_data(body)),
                        None => request.respond(tiny_http::Response::empty(404)),
                    };
                }
            })
        };
        Self {
            server,
            base,
            routes,
            requests,
            thread: Some(thread),
        }
    }

    /// Serves `body` at `path` (e.g. "/slang.zip") and returns its full URL.
    pub fn serve(&self, path: &str, body: impl Into<Vec<u8>>) -> String {
        self.routes
            .lock()
            .unwrap()
            .insert(path.to_owned(), body.into());
        self.url(path)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
