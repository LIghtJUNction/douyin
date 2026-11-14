//! Developer probe: private input stays on stdin; stdout contains booleans only.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::Read;

fn run() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .take(16385)
        .read_to_string(&mut input)
        .map_err(|_| "读取探测输入失败")?;
    if input.len() > 16384 {
        return Err("探测输入超过限制".into());
    }
    let snapshot: Value = serde_json::from_str(&input).map_err(|_| "探测输入不是合法 JSON")?;
    let request = if let Some(request) = snapshot.get("request") {
        Some((
            request["ticket"].as_str().ok_or("探测请求缺少 ticket")?,
            request["path"].as_str().ok_or("探测请求缺少路径")?,
            request["timestamp"].as_u64().ok_or("探测请求时间戳无效")?,
        ))
    } else if snapshot["request_probe"] == true {
        Some(("fixture-ticket", "/aweme/v1/feed/", 1700000000))
    } else {
        None
    };
    let message = b"douyin-cli-guard-probe-v1";
    let peer = snapshot["native_signature"]
        .as_str()
        .map(|s| STANDARD.decode(s))
        .transpose()
        .map_err(|_| "原生签名编码无效")?;
    let (local_verified, native_verified) = if snapshot["hmac_probe"] == true {
        let signer = douyin_cli::guard::HmacSigner::from_snapshot(&snapshot)?;
        let signature = if let Some((ticket, path, timestamp)) = request {
            signer.sign_request(ticket, path, timestamp)?
        } else {
            signer.sign(message)
        };
        (
            request.map_or_else(
                || signer.verify(message, &signature),
                |(ticket, path, timestamp)| {
                    signer.verify_request(ticket, path, timestamp, &signature)
                },
            ),
            peer.map(|sig| {
                request.map_or_else(
                    || signer.verify(message, &sig),
                    |(ticket, path, timestamp)| {
                        signer.verify_request(ticket, path, timestamp, &sig)
                    },
                )
            }),
        )
    } else {
        let signer = douyin_cli::guard::ReeSigner::from_snapshot(&snapshot)?;
        let signature = if let Some((ticket, path, timestamp)) = request {
            signer.sign_request(ticket, path, timestamp)?
        } else {
            signer.sign(message)?
        };
        (
            request.map_or_else(
                || signer.verify(message, &signature),
                |(ticket, path, timestamp)| {
                    signer.verify_request(ticket, path, timestamp, &signature)
                },
            ),
            peer.map(|sig| {
                request.map_or_else(
                    || signer.verify(message, &sig),
                    |(ticket, path, timestamp)| {
                        signer.verify_request(ticket, path, timestamp, &sig)
                    },
                )
            }),
        )
    };
    println!(
        "{}",
        json!({"local_signature_verified":local_verified,
        "native_signature_verified":native_verified})
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
