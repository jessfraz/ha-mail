#![allow(dead_code)]

use ha_mail::stable::{
    Address, Connection, Edge, Location, Onboarding, PageInfo, ProviderItem, Scan,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub fn location(id: &str) -> Location {
    Location {
        id: id.into(),
        status: Some("active".into()),
        address: Address {
            line1: "Example Street".into(),
            line2: Some("Example mailbox".into()),
            city: "Example City".into(),
            state: "EX".into(),
            postal_code: "00000".into(),
        },
        onboarding: Some(Onboarding {
            status: "completed".into(),
        }),
    }
}
pub fn item(id: &str, location_id: &str, status: Option<&str>) -> ProviderItem {
    ProviderItem {
        id: id.into(),
        sender: "Example Sender".into(),
        location: ha_mail::stable::LocationRef {
            id: location_id.into(),
        },
        created_at: Some("2026-01-01T12:00:00Z".into()),
        archived_at: None,
        read_at: None,
        clear_at: Some("2026-12-31T12:00:00Z".into()),
        image_url: Some("https://assets.usestable.com/envelope".into()),
        shred_details: None,
        scan_details: status.map(|status| Scan {
            status: status.into(),
            image_url: Some("https://assets.usestable.com/document".into()),
            summary: Some("Example summary".into()),
            scan_notice_type: None,
            ocr_result_urls: None,
        }),
    }
}
pub fn connection<T>(nodes: Vec<T>, next: Option<&str>) -> Connection<T> {
    Connection {
        edges: nodes.into_iter().map(|node| Edge { node }).collect(),
        page_info: PageInfo {
            end_cursor: next.map(str::to_owned),
            has_next_page: next.is_some(),
        },
    }
}

pub struct Fixture {
    pub origin: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub request_times: Arc<Mutex<Vec<std::time::Instant>>>,
    pub items: Arc<Mutex<Vec<ProviderItem>>>,
    pub fault: Arc<Mutex<Option<(u16, String)>>>,
    pub retry_after: Arc<Mutex<u64>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    pub fn new(items: Vec<ProviderItem>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_times = Arc::new(Mutex::new(Vec::new()));
        let values = Arc::new(Mutex::new(items));
        let fault = Arc::new(Mutex::new(None::<(u16, String)>));
        let retry_after = Arc::new(Mutex::new(1800));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, times, data, fail, retry, done) = (
            requests.clone(),
            request_times.clone(),
            values.clone(),
            fault.clone(),
            retry_after.clone(),
            stop.clone(),
        );
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 16 * 1024);
                }
                let request = String::from_utf8(request).unwrap();
                let target = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let target = reqwest::Url::parse(&format!("http://fixture{target}")).unwrap();
                seen.lock().unwrap().push(request.clone());
                times.lock().unwrap().push(std::time::Instant::now());
                let (status, body) = if let Some((status, body)) = fail.lock().unwrap().clone() {
                    (status, body)
                } else if target.path() == "/v1/locations" {
                    let id = target
                        .query_pairs()
                        .find(|(k, _)| k == "id")
                        .map(|(_, v)| v.into_owned())
                        .unwrap_or_else(|| "location-a".into());
                    (
                        200,
                        serde_json::to_string(&connection(vec![location(&id)], None)).unwrap(),
                    )
                } else if target.path() == "/v1/mail-items" {
                    let archived = target
                        .query_pairs()
                        .find(|(k, _)| k == "archived")
                        .is_some_and(|(_, v)| v == "true");
                    let nodes = data
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|item| item.archived_at.is_some() == archived)
                        .cloned()
                        .collect();
                    (
                        200,
                        serde_json::to_string(&connection(nodes, None)).unwrap(),
                    )
                } else if let Some(id) = target.path().strip_prefix("/v1/mail-items/") {
                    match data.lock().unwrap().iter().find(|item| item.id == id) {
                        Some(item) => (200, serde_json::to_string(item).unwrap()),
                        None => (404, "{}".into()),
                    }
                } else {
                    (404, "{}".into())
                };
                let extra = if status == 429 {
                    format!("Retry-After: {}\r\n", retry.lock().unwrap())
                } else {
                    String::new()
                };
                write!(stream,"HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            origin,
            requests,
            request_times,
            items: values,
            fault,
            retry_after,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.expect("fixture thread completed");
        }
    }
}
