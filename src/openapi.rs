use std::collections::HashMap;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

pub const BASE_URL: &str = "https://open.douyin.com";

pub struct OpenApiClient {
    base_url: String,
    client: Client,
}

impl OpenApiClient {
    pub fn new() -> Result<Self, String> {
        Self::with_base_url(BASE_URL)
    }

    pub fn with_base_url(base_url: &str) -> Result<Self, String> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            client,
        })
    }

    pub fn authorize_url(
        &self,
        client_key: &str,
        redirect_uri: &str,
        scopes: &[String],
        state: Option<&str>,
    ) -> Result<String, String> {
        let mut url =
            Url::parse(&self.url("/platform/oauth/connect/")).map_err(|error| error.to_string())?;
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
        let mut request = self.client.request(method, self.url(spec.path));
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

        let response = request.send().map_err(|error| error.to_string())?;
        let status = response.status();
        let text = response.text().map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!("OpenAPI HTTP 请求失败: {status} {text}"));
        }
        let data: Value =
            serde_json::from_str(&text).map_err(|_| format!("OpenAPI 响应不是 JSON: {text}"))?;
        if !data.is_object() {
            return Err("OpenAPI 响应不是 JSON object".to_owned());
        }
        Ok(data)
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_owned()
        } else {
            format!("{}/{}", self.base_url, path.trim_start_matches('/'))
        }
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
    message_type: &str,
    content: Value,
    persona_id: Option<&str>,
    client_msg_id: Option<&str>,
) -> Value {
    let mut body = Map::from_iter([
        ("to_user_id".to_owned(), json!(to_user_id)),
        ("message_type".to_owned(), json!(message_type)),
        ("content".to_owned(), json!(content.to_string())),
    ]);
    if let Some(value) = persona_id {
        body.insert("persona_id".to_owned(), json!(value));
    }
    if let Some(value) = client_msg_id {
        body.insert("client_msg_id".to_owned(), json!(value));
    }
    Value::Object(body)
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
        let key = HeaderName::try_from(key).map_err(|error| error.to_string())?;
        let value = HeaderValue::try_from(value).map_err(|error| error.to_string())?;
        values.insert(key, value);
    }
    request = request.headers(values);
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::{OpenApiClient, RequestSpec, im_message_body};
    use serde_json::json;

    #[test]
    fn authorize_url_encodes_values() {
        let client = OpenApiClient::new().unwrap();
        let url = client
            .authorize_url(
                "client",
                "https://example.com/callback",
                &["user_info".to_owned(), "item.comment".to_owned()],
                Some("state value"),
            )
            .unwrap();
        assert!(url.starts_with("https://open.douyin.com/platform/oauth/connect/?"));
        assert!(url.contains("scope=user_info%2Citem.comment"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fexample.com%2Fcallback"));
        assert!(url.contains("state=state+value"));
    }

    #[test]
    fn request_rejects_missing_token_before_network() {
        let client = OpenApiClient::new().unwrap();
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
    fn im_body_serializes_content_as_compact_json_string() {
        let body = im_message_body(
            "user",
            "text",
            json!({"text": "你好"}),
            None,
            Some("client-msg"),
        );
        assert_eq!(body["content"], "{\"text\":\"你好\"}");
        assert_eq!(body["client_msg_id"], "client-msg");
    }
}
