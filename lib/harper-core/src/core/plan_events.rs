// Copyright 2026 coccinella-labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::core::plan::PlanState;
use ring::digest::{SHA256, digest};
use std::collections::HashSet;
use std::fs;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct PlanUpdateEvent {
    pub event_id: i64,
    pub session_id: String,
    pub plan: Option<PlanState>,
}

static PLAN_UPDATES: OnceLock<broadcast::Sender<PlanUpdateEvent>> = OnceLock::new();
static PLAN_EVENT_LISTENERS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn sender() -> &'static broadcast::Sender<PlanUpdateEvent> {
    PLAN_UPDATES.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(256);
        tx
    })
}

pub fn subscribe() -> broadcast::Receiver<PlanUpdateEvent> {
    sender().subscribe()
}

pub fn notify(event_id: i64, session_id: &str, plan: Option<PlanState>) {
    let _ = sender().send(PlanUpdateEvent {
        event_id,
        session_id: session_id.to_string(),
        plan,
    });
}

fn listener_registry() -> &'static Mutex<HashSet<String>> {
    PLAN_EVENT_LISTENERS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn registry_dir(db_key: &str) -> PathBuf {
    let digest = digest(&SHA256, db_key.as_bytes());
    let hex = digest
        .as_ref()
        .iter()
        .take(8)
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    std::env::temp_dir().join("hp-plan").join(hex)
}

fn listener_registry_path(db_key: &str, port: u16) -> PathBuf {
    registry_dir(db_key).join(format!("{}.port", port))
}

fn signal_file_path(db_key: &str) -> PathBuf {
    registry_dir(db_key).join("signal")
}

fn parse_signal_payload(payload: &str) -> Option<(i64, String)> {
    let (event_id, session_id) = payload.trim().split_once('\t')?;
    let event_id = event_id.parse::<i64>().ok()?;
    Some((event_id, session_id.to_string()))
}

pub fn ensure_cross_process_listener(db_key: &str) -> bool {
    let mut started = listener_registry()
        .lock()
        .expect("plan event listener registry lock");
    if !started.insert(db_key.to_string()) {
        return true;
    }
    drop(started);

    let dir = registry_dir(db_key);
    if fs::create_dir_all(&dir).is_err() {
        let mut started = listener_registry()
            .lock()
            .expect("plan event listener registry lock");
        started.remove(db_key);
        return false;
    }

    let signal_path = signal_file_path(db_key);
    std::thread::spawn(move || {
        let mut last_seen_event_id: Option<i64> = None;
        loop {
            if let Ok(payload) = fs::read_to_string(&signal_path) {
                if let Some((event_id, session_id)) = parse_signal_payload(&payload) {
                    if Some(event_id) > last_seen_event_id {
                        last_seen_event_id = Some(event_id);
                        notify(event_id, &session_id, None);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });

    let Ok(socket) = UdpSocket::bind(("127.0.0.1", 0)) else {
        return true;
    };
    let Ok(address) = socket.local_addr() else {
        return true;
    };
    let registry_path = listener_registry_path(db_key, address.port());
    let _ = fs::write(&registry_path, address.port().to_string());
    std::thread::spawn(move || {
        let mut buf = [0_u8; 1024];
        while let Ok(size) = socket.recv(&mut buf) {
            let Ok(payload) = std::str::from_utf8(&buf[..size]) else {
                continue;
            };
            let Some((event_id, session_id)) = parse_signal_payload(payload) else {
                continue;
            };
            notify(event_id, &session_id, None);
        }
        let _ = fs::remove_file(&registry_path);
    });
    true
}

pub fn notify_cross_process(db_key: &str, event_id: i64, session_id: &str) {
    let dir = registry_dir(db_key);
    let signal_path = signal_file_path(db_key);
    let payload = format!("{}\t{}", event_id, session_id);
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&signal_path, &payload);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let Ok(sender) = UdpSocket::bind(("127.0.0.1", 0)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("port") {
            continue;
        }
        let Ok(port_text) = fs::read_to_string(&path) else {
            let _ = fs::remove_file(path);
            continue;
        };
        let Ok(port) = port_text.trim().parse::<u16>() else {
            let _ = fs::remove_file(path);
            continue;
        };
        if sender
            .send_to(payload.as_bytes(), ("127.0.0.1", port))
            .is_err()
        {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_cross_process_listener, listener_registry_path, notify, notify_cross_process,
        parse_signal_payload, signal_file_path, subscribe,
    };
    use crate::core::plan::PlanState;
    use std::fs;
    use std::net::UdpSocket;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_db_key(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        format!("{}-{}-{}", prefix, std::process::id(), nanos)
    }

    #[tokio::test]
    async fn broadcasts_plan_updates() {
        let mut rx = subscribe();
        let plan = PlanState::default();

        notify(42, "session-a", Some(plan.clone()));

        let event = rx.recv().await.expect("event");
        assert_eq!(event.event_id, 42);
        assert_eq!(event.session_id, "session-a");
        assert_eq!(event.plan, Some(plan));
    }

    #[test]
    fn cross_process_notifier_sends_socket_payload() {
        let db_key = unique_test_db_key("plan-event-test");
        let socket = match UdpSocket::bind(("127.0.0.1", 0)) {
            Ok(socket) => socket,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                return;
            }
            Err(err) => panic!("bind test listener: {}", err),
        };
        let port = socket.local_addr().expect("listener address").port();
        let path = listener_registry_path(&db_key, port);
        fs::create_dir_all(path.parent().expect("registry dir")).expect("create notifier dir");
        fs::write(&path, port.to_string()).expect("write notifier port");

        notify_cross_process(&db_key, 7, "session-z");

        let mut buf = [0_u8; 256];
        let size = socket.recv(&mut buf).expect("receive notifier payload");
        let payload = std::str::from_utf8(&buf[..size]).expect("utf8 payload");
        assert_eq!(payload, "7\tsession-z");

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn cross_process_listener_rebroadcasts_updates_to_subscribers() {
        let db_key = unique_test_db_key("plan-listener-test");
        assert!(ensure_cross_process_listener(&db_key));
        let mut rx = subscribe();

        notify_cross_process(&db_key, 9, "session-b");

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let event = rx.recv().await.expect("cross-process event");
                if event.event_id == 9 && event.session_id == "session-b" {
                    break event;
                }
            }
        })
        .await
        .expect("timeout waiting for cross-process event");
        assert_eq!(event.event_id, 9);
        assert_eq!(event.session_id, "session-b");
        assert_eq!(event.plan, None);
    }

    #[test]
    fn parse_signal_payload_reads_event_and_session() {
        let parsed = parse_signal_payload("41\tsession-y\n").expect("payload");
        assert_eq!(parsed.0, 41);
        assert_eq!(parsed.1, "session-y");
    }

    #[test]
    fn notify_cross_process_writes_signal_file() {
        let db_key = unique_test_db_key("plan-signal-file-test");

        notify_cross_process(&db_key, 11, "session-c");

        let signal_path = signal_file_path(&db_key);
        let payload = fs::read_to_string(signal_path).expect("signal payload");
        assert_eq!(payload, "11\tsession-c");
    }
}
