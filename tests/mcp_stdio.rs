use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};

struct Mcp {
    child: Child,
    input: ChildStdin,
    output: Receiver<Result<Value, String>>,
    inline: bool,
    _home: tempfile::TempDir,
}

impl Mcp {
    fn start(version: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_douyin"))
            .arg("mcp")
            .env("DOUYIN_HOME", home.path())
            .env_remove("DOUYIN_ACCESS_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = line
                    .map_err(|_| "stdio read failed".to_owned())
                    .and_then(|line| {
                        serde_json::from_str(&line)
                            .map_err(|_| "stdout contained non-JSON data".into())
                    });
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let mut mcp = Self {
            child,
            input,
            output,
            inline: version == "2026-07-28",
            _home: home,
        };
        // MCP 2026-07-28 carries lifecycle metadata per request and no longer
        // negotiates through initialize. Keep the legacy handshake test too.
        if mcp.inline {
            return mcp;
        }
        let initialized=mcp.call(1,"initialize",json!({"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"local-test","version":"1"}}));
        assert_eq!(initialized["result"]["protocolVersion"], version);
        mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        mcp
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }
    fn call(&mut self, id: u64, method: &str, mut params: Value) -> Value {
        if self.inline {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"local-test","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}});
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let response = self
                .output
                .recv_timeout(Duration::from_secs(8))
                .expect("MCP response timed out")
                .unwrap();
            if response.get("id") == Some(&json!(id)) {
                return response;
            }
        }
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_negotiates_legacy_protocol_and_executes_real_offline_tool() {
    let mut mcp = Mcp::start("2025-11-25");
    let list = mcp.call(2, "tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    for name in [
        "hot_words",
        "hot_memes",
        "demand_discovery",
        "auth_status",
        "userinfo",
        "comment_list",
        "comment_replies",
        "comment_reply",
        "im_message_send",
        "openapi_request",
        "native_me",
        "native_search",
        "native_aweme",
        "native_comments",
        "native_comment_replies",
        "native_user_posts",
        "native_live_permissions",
    ] {
        assert!(
            tools.iter().any(|tool| tool["name"] == name),
            "missing {name}"
        );
    }
    let result = mcp.call(
        3,
        "tools/call",
        json!({"name":"hot_words","arguments":{"texts":["rust","rust"],"min_count":2}}),
    );
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(
        result["result"]["structuredContent"]["items"][0]["text"],
        "rust"
    );
    let unknown = mcp.call(
        4,
        "tools/call",
        json!({"name":"does-not-exist","arguments":{}}),
    );
    assert!(unknown.get("error").is_some());
    let unconfigured = mcp.call(
        6,
        "tools/call",
        json!({"name":"native_live_permissions","arguments":{}}),
    );
    assert_eq!(unconfigured["result"]["isError"], true);
    let denied=mcp.call(5,"tools/call",json!({"name":"comment_reply","arguments":{"item_id":"fixture-item","content":"fixture-comment","confirmed":false}}));
    assert_eq!(denied["result"]["isError"], true);
    assert!(
        denied["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("confirmed=true")
    );
}

#[test]
fn stdio_also_supports_current_protocol() {
    let mut mcp = Mcp::start("2026-07-28");
    let response = mcp.call(2, "tools/list", json!({}));
    assert!(
        response["result"]["tools"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    let response = mcp.call(
        3,
        "tools/call",
        json!({"name":"hot_words","arguments":{"texts":["rust","rust"]}}),
    );
    assert_eq!(response["result"]["isError"], false);
}
