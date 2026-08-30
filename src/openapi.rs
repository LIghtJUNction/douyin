use std::collections::HashMap;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::err;

pub const BASE_URL: &str = "https://open.douyin.com";

pub struct OpenApiClient {
    base_url: Url,
    client: Client,
}

impl OpenApiClient {
    pub fn new() -> Result<Self, String> {
        Self::with_base_url(BASE_URL)
    }

    pub fn with_base_url(base_url: &str) -> Result<Self, String> {
        let mut base_url =
            Url::parse(base_url).map_err(|error| format!("OpenAPI base URL 无效: {error}"))?;
        if !matches!(base_url.scheme(), "http" | "https") || base_url.host_str().is_none() {
            return Err("OpenAPI base URL 必须是有效的 HTTP(S) 地址".to_owned());
        }
        base_url.set_path("/");
        base_url.set_query(None);
        base_url.set_fragment(None);
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(err)?;
        Ok(Self { base_url, client })
    }

    pub fn authorize_url(
        &self,
        client_key: &str,
        redirect_uri: &str,
        scopes: &[String],
        state: Option<&str>,
    ) -> Result<String, String> {
        let mut url = self.url("/platform/oauth/connect/")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_key", client_key);
            query.append_pair("response_type", "code");
            query.append_pair("scope", &scopes.join(","));
            query.append_pair("redirect_uri", redirect_uri);
            if let Some(state) = state {
                query.append_pair("state", state);
            }
        }
        Ok(url.into())
    }

    pub fn client_token(&self, client_key: &str, client_secret: &str) -> Result<Value, String> {
        self.request(RequestSpec {
            method: "POST",
            path: "/oauth/client_token/",
            form: Some(HashMap::from([
                ("client_key".to_owned(), client_key.to_owned()),
                ("client_secret".to_owned(), client_secret.to_owned()),
                ("grant_type".to_owned(), "client_credential".to_owned()),
            ])),
            auth_required: false,
            ..RequestSpec::default()
        })
    }

    pub fn access_token(
        &self,
        client_key: &str,
        client_secret: &str,
        code: &str,
    ) -> Result<Value, String> {
        self.request(RequestSpec {
            method: "POST",
            path: "/oauth/access_token/",
            form: Some(HashMap::from([
                ("client_key".to_owned(), client_key.to_owned()),
                ("client_secret".to_owned(), client_secret.to_owned()),
                ("code".to_owned(), code.to_owned()),
                ("grant_type".to_owned(), "authorization_code".to_owned()),
            ])),
            auth_required: false,
            ..RequestSpec::default()
        })
    }

    pub fn refresh_token(&self, client_key: &str, refresh_token: &str) -> Result<Value, String> {
        self.request(RequestSpec {
            method: "POST",
            path: "/oauth/refresh_token/",
            form: Some(HashMap::from([
                ("client_key".to_owned(), client_key.to_owned()),
                ("grant_type".to_owned(), "refresh_token".to_owned()),
                ("refresh_token".to_owned(), refresh_token.to_owned()),
            ])),
            auth_required: false,
            ..RequestSpec::default()
        })
    }

    pub fn renew_refresh_token(
        &self,
        client_key: &str,
        refresh_token: &str,
    ) -> Result<Value, String> {
        self.request(RequestSpec {
            method: "POST",
            path: "/oauth/renew_refresh_token/",
            form: Some(HashMap::from([
                ("client_key".to_owned(), client_key.to_owned()),
                ("refresh_token".to_owned(), refresh_token.to_owned()),
            ])),
            auth_required: false,
            ..RequestSpec::default()
        })
    }

    pub fn request(&self, spec: RequestSpec<'_>) -> Result<Value, String> {
        if spec.auth_required && spec.token.is_none_or(str::is_empty) {
            return Err("调用 OpenAPI 需要 access-token 或 client-token".to_owned());
        }
        let method = spec
            .method
            .parse::<reqwest::Method>()
            .map_err(|error| format!("HTTP method 无效: {error}"))?;
        let url = self.url(spec.path)?;
        let mut request = self.client.request(method, url);
        if let Some(token) = spec.token {
            request = request.header("access-token", token);
        }
        request = add_headers(request, spec.headers)?;
        if let Some(params) = spec.params {
            request = request.query(&params);
        }
        if let Some(form) = spec.form {
            request = request.form(&form);
        } else if let Some(body) = spec.json_body {
            request = request.json(&body);
        }

        let response = request.send().map_err(err)?;
        let status = response.status();
        let text = response.text().map_err(err)?;
        if !status.is_success() {
            return Err(format!(
                "OpenAPI HTTP 请求失败: {status} {}",
                body_excerpt(&text)
            ));
        }
        let data: Value = serde_json::from_str(&text)
            .map_err(|_| format!("OpenAPI 响应不是 JSON: {}", body_excerpt(&text)))?;
        if !data.is_object() {
            return Err("OpenAPI 响应不是 JSON object".to_owned());
        }
        if let Some(error) = api_error(&data) {
            return Err(error);
        }
        Ok(data)
    }

    fn url(&self, path: &str) -> Result<Url, String> {
        let resolved = self
            .base_url
            .join(path)
            .map_err(|error| format!("OpenAPI path 无效: {error}"))?;
        if !same_origin(&self.base_url, &resolved) {
            return Err(format!(
                "拒绝跨域 OpenAPI 请求: {}",
                resolved.origin().ascii_serialization()
            ));
        }
        Ok(resolved)
    }
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn body_excerpt(body: &str) -> String {
    const MAX_CHARS: usize = 2_000;
    let mut characters = body.chars();
    let excerpt: String = characters.by_ref().take(MAX_CHARS).collect();
    if characters.next().is_some() {
        format!("{excerpt}…（响应已截断）")
    } else {
        excerpt
    }
}

#[derive(Default)]
pub struct RequestSpec<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub token: Option<&'a str>,
    pub params: Option<HashMap<String, String>>,
    pub json_body: Option<Value>,
    pub form: Option<HashMap<String, String>>,
    pub headers: Option<HashMap<String, String>>,
    pub auth_required: bool,
}

pub fn im_message_body(
    to_user_id: &str,
    scene: &str,
    msg_id: &str,
    conversation_id: &str,
    content: Value,
) -> Value {
    json!({
        "to_user_id": to_user_id,
        "scene": scene,
        "msg_id": msg_id,
        "conversation_id": conversation_id,
        "content": content,
    })
}

fn api_error(response: &Value) -> Option<String> {
    let candidates = [
        (response.get("err_no"), response),
        (response.get("error_code"), response),
        (response.get("status_code"), response),
        (
            response.pointer("/data/error_code"),
            response.get("data").unwrap_or(response),
        ),
        (
            response.pointer("/extra/error_code"),
            response.get("extra").unwrap_or(response),
        ),
    ];
    let (code, details) = candidates
        .into_iter()
        .find_map(|(code, details)| nonzero_code(code?).map(|code| (code, details)))?;
    let message = [
        "err_msg",
        "description",
        "error_msg",
        "message",
        "sub_description",
    ]
    .into_iter()
    .find_map(|key| details.get(key).and_then(Value::as_str))
    .filter(|message| !message.trim().is_empty())
    .unwrap_or("未提供错误说明");
    Some(format!("OpenAPI 返回业务错误 {code}: {message}"))
}

fn nonzero_code(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) if number.as_i64().is_some_and(|code| code != 0) => {
            Some(number.to_string())
        }
        Value::String(code) if !code.is_empty() && code != "0" => Some(code.clone()),
        _ => None,
    }
}

fn add_headers(
    mut request: RequestBuilder,
    headers: Option<HashMap<String, String>>,
) -> Result<RequestBuilder, String> {
    let Some(headers) = headers else {
        return Ok(request);
    };
    let mut values = HeaderMap::new();
    for (key, value) in headers {
        let key = HeaderName::try_from(key).map_err(err)?;
        let value = HeaderValue::try_from(value).map_err(err)?;
        values.insert(key, value);
    }
    request = request.headers(values);
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::{OpenApiClient, RequestSpec, api_error, body_excerpt, im_message_body};
    use crate::test_support::must;
    use serde_json::json;

    #[test]
    fn authorize_url_encodes_values() {
        let client = must(OpenApiClient::new());
        let url = must(client.authorize_url(
            "client",
            "https://example.com/callback",
            &["user_info".to_owned(), "item.comment".to_owned()],
            Some("state value"),
        ));
        assert!(url.starts_with("https://open.douyin.com/platform/oauth/connect/?"));
        assert!(url.contains("scope=user_info%2Citem.comment"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fexample.com%2Fcallback"));
        assert!(url.contains("state=state+value"));
    }

    #[test]
    fn request_rejects_missing_token_before_network() {
        let client = must(OpenApiClient::new());
        let error = client
            .request(RequestSpec {
                method: "GET",
                path: "/oauth/userinfo/",
                auth_required: true,
                ..RequestSpec::default()
            })
            .unwrap_err();
        assert!(error.contains("access-token"));
    }

    #[test]
    fn request_rejects_cross_origin_url_before_network() {
        let client = must(OpenApiClient::new());
        let error = client
            .request(RequestSpec {
                method: "GET",
                path: "https://example.com/collect",
                token: Some("secret-token"),
                auth_required: true,
                ..RequestSpec::default()
            })
            .unwrap_err();
        assert!(error.contains("拒绝跨域"));
        assert!(!error.contains("secret-token"));
    }

    #[test]
    fn validates_base_url_and_bounds_error_bodies() {
        assert!(OpenApiClient::with_base_url("file:///tmp/api").is_err());
        assert_eq!(body_excerpt("short"), "short");
        let excerpt = body_excerpt(&"界".repeat(2_001));
        assert!(excerpt.ends_with("…（响应已截断）"));
        assert_eq!(excerpt.chars().count(), 2_008);
    }

    #[test]
    fn im_body_uses_the_current_direct_message_shape() {
        let body = im_message_body(
            "user",
            "im_reply_msg",
            "message-id",
            "conversation-id",
            json!({"msg_type": 1, "text": {"text": "你好"}}),
        );
        assert_eq!(body["scene"], "im_reply_msg");
        assert_eq!(body["msg_id"], "message-id");
        assert_eq!(body["conversation_id"], "conversation-id");
        assert_eq!(body["content"]["msg_type"], 1);
        assert_eq!(body["content"]["text"]["text"], "你好");
    }

    #[test]
    fn detects_current_and_legacy_business_errors() {
        assert_eq!(
            api_error(&json!({"err_no": 2100005, "err_msg": "参数不合法"})).as_deref(),
            Some("OpenAPI 返回业务错误 2100005: 参数不合法")
        );
        assert_eq!(
            api_error(
                &json!({"data": {"error_code": "10008", "description": "access_token 无效"}})
            )
            .as_deref(),
            Some("OpenAPI 返回业务错误 10008: access_token 无效")
        );
        assert!(api_error(&json!({"err_no": 0, "data": {"error_code": 0}})).is_none());
    }
}
