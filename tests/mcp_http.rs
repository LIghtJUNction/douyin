use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use tiny_http::{Response, Server};

struct HttpMcp {
    child: Child,
    base_url: String,
    _home: tempfile::TempDir,
}

impl HttpMcp {
    fn start(extra_args: &[&str]) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_douyin"));
        command
            .arg("mcp")
            .arg("--listen")
            .arg("127.0.0.1:0")
            .args(extra_args)
            .env("DOUYIN_HOME", home.path())
            .env_remove("DOUYIN_ACCESS_TOKEN")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(url) = line.strip_prefix("MCP Streamable HTTP: ") {
                    let _ = tx.send(url.trim_end_matches("/mcp").to_owned());
                }
            }
        });
        let base_url = rx
            .recv_timeout(Duration::from_secs(8))
            .expect("MCP HTTP server did not report listening address");
        Self {
            child,
            base_url,
            _home: home,
        }
    }
}

impl Drop for HttpMcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_mcp_response(response: reqwest::blocking::Response) -> Value {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = response.text().unwrap();
    if content_type.contains("text/event-stream") {
        for line in body.lines() {
            if let Some(data) = line.strip_prefix("data: ")
                && !data.trim().is_empty()
            {
                return serde_json::from_str(data).unwrap();
            }
        }
        panic!("SSE response contained no JSON data: {body}");
    }
    serde_json::from_str(&body).unwrap()
}

#[test]
fn streamable_http_serves_mcp_tools_over_loopback() {
    let server = HttpMcp::start(&[]);
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();
    let mcp_url = format!("{}/mcp", server.base_url);

    let init_resp = client
        .post(&mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "local-http-test", "version": "1"}
            }
        }))
        .send()
        .unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    let session_id = init_resp
        .headers()
        .get("mcp-session-id")
        .expect("missing mcp-session-id header")
        .to_str()
        .unwrap()
        .to_owned();
    let init_json = parse_mcp_response(init_resp);
    assert_eq!(init_json["id"], 1);
    assert!(init_json["result"]["serverInfo"]["name"].is_string());

    let notify_resp = client
        .post(&mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .send()
        .unwrap();
    assert!(notify_resp.status().is_success());

    let list_resp = client
        .post(&mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_json = parse_mcp_response(list_resp);
    let tools = list_json["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "auth_status"));

    let call_resp = client
        .post(&mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "auth_status",
                "arguments": {}
            }
        }))
        .send()
        .unwrap();
    assert_eq!(call_resp.status(), StatusCode::OK);
    let call_json = parse_mcp_response(call_resp);
    assert_eq!(call_json["id"], 3);
    assert_eq!(call_json["result"]["isError"], false);

    // Without --webhook, /events returns 503.
    let unconfigured = client
        .post(format!("{}/events", server.base_url))
        .json(&json!({"type": "im.message", "data": {"text": "hi"}}))
        .send()
        .unwrap();
    assert_eq!(unconfigured.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn events_endpoint_authenticates_and_forwards_to_webhook() {
    let webhook_server = Server::http("127.0.0.1:0").unwrap();
    let webhook_addr = webhook_server.server_addr().to_ip().unwrap();
    let webhook_url = format!("http://{webhook_addr}/hook");
    let (hook_tx, hook_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for index in 0..2 {
            let mut req = webhook_server.recv().unwrap();
            let auth = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .map(|h| h.value.as_str().to_owned());
            let mut body = String::new();
            req.as_reader().read_to_string(&mut body).unwrap();
            let payload: Value = serde_json::from_str(&body).unwrap();
            let _ = hook_tx.send((auth, payload));
            let status = if index == 0 { 200 } else { 500 };
            let _ = req.respond(Response::from_string("ok").with_status_code(status));
        }
    });

    let server = HttpMcp::start(&[
        "--webhook",
        &webhook_url,
        "--event-token",
        "in-secret",
        "--webhook-token",
        "out-secret",
    ]);
    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();
    let events_url = format!("{}/events", server.base_url);

    // Missing or wrong token -> 401.
    let unauth = client
        .post(&events_url)
        .json(&json!({"type": "im.message", "data": {"text": "hi"}}))
        .send()
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    // Invalid event type -> 400.
    let bad_event = client
        .post(&events_url)
        .bearer_auth("in-secret")
        .json(&json!({"type": "bad type"}))
        .send()
        .unwrap();
    assert_eq!(bad_event.status(), StatusCode::BAD_REQUEST);

    // Valid event -> 202 and forwarded with webhook Bearer token.
    let accepted = client
        .post(&events_url)
        .bearer_auth("in-secret")
        .json(&json!({"type": "im.message", "data": {"text": "hello"}}))
        .send()
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let accepted_json: Value = accepted.json().unwrap();
    assert_eq!(accepted_json["accepted"], true);
    let event_id = accepted_json["id"].as_str().unwrap().to_owned();
    assert_eq!(event_id.len(), 32);

    let (auth, forwarded) = hook_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(auth.as_deref(), Some("Bearer out-secret"));
    assert_eq!(forwarded["id"], event_id);
    assert_eq!(forwarded["type"], "im.message");
    assert_eq!(forwarded["source"], "douyin-mcp");
    assert_eq!(forwarded["data"]["text"], "hello");
    assert!(forwarded["timestamp"].as_u64().is_some());

    // Downstream webhook error -> 502.
    let failed = client
        .post(&events_url)
        .bearer_auth("in-secret")
        .json(&json!({"type": "notification", "data": {"count": 1}}))
        .send()
        .unwrap();
    assert_eq!(failed.status(), StatusCode::BAD_GATEWAY);
    let failed_json: Value = failed.json().unwrap();
    assert_eq!(failed_json["accepted"], false);
}

#[test]
fn mcp_cli_rejects_invalid_listen_and_webhook_flag_combinations() {
    for args in [
        &["mcp", "--listen", "0.0.0.0:8788"][..],
        &["mcp", "--webhook", "https://example.com/events"][..],
        &["mcp", "--listen", "127.0.0.1:0", "--event-token", "tok"][..],
        &["mcp", "--listen", "127.0.0.1:0", "--webhook-token", "tok"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_douyin"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "expected failure for args: {args:?}"
        );
    }
}
