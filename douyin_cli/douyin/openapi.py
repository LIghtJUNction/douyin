"""Douyin OpenAPI client."""

from __future__ import annotations

from typing import Any, Self
from urllib.parse import urlencode

import niquests as requests
import ujson as json

OPENAPI_BASE_URL = "https://open.douyin.com"


class DouyinOpenAPIError(RuntimeError):
    """Raised when the official OpenAPI request fails."""


class DouyinOpenAPIClient:
    """Small HTTP client for Douyin official OpenAPI endpoints."""

    def __init__(self, base_url: str = OPENAPI_BASE_URL) -> None:
        self.base_url = base_url.rstrip("/")
        self._session = requests.Session()

    def close(self) -> None:
        self._session.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
        self.close()
        return False

    def client_token(self, client_key: str, client_secret: str) -> dict[str, Any]:
        """Generate a client_token for APIs that do not require user auth."""
        return self.request(
            "POST",
            "/oauth/client_token/",
            form={
                "client_key": client_key,
                "client_secret": client_secret,
                "grant_type": "client_credential",
            },
            auth_required=False,
        )

    def authorize_url(
        self,
        client_key: str,
        redirect_uri: str,
        scopes: list[str],
        state: str | None = None,
    ) -> str:
        """Build the official OAuth authorization URL."""
        params = {
            "client_key": client_key,
            "response_type": "code",
            "scope": ",".join(scopes),
            "redirect_uri": redirect_uri,
        }
        if state:
            params["state"] = state
        return f"{self._url('/platform/oauth/connect/')}?{urlencode(params)}"

    def access_token(
        self,
        client_key: str,
        client_secret: str,
        code: str,
    ) -> dict[str, Any]:
        """Exchange an authorization code for a user access_token."""
        return self.request(
            "POST",
            "/oauth/access_token/",
            form={
                "client_key": client_key,
                "client_secret": client_secret,
                "code": code,
                "grant_type": "authorization_code",
            },
            auth_required=False,
        )

    def refresh_token(self, client_key: str, refresh_token: str) -> dict[str, Any]:
        """Refresh a user access_token with a refresh_token."""
        return self.request(
            "POST",
            "/oauth/refresh_token/",
            form={
                "client_key": client_key,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            },
            auth_required=False,
        )

    def renew_refresh_token(
        self,
        client_key: str,
        refresh_token: str,
    ) -> dict[str, Any]:
        """Renew a user refresh_token."""
        return self.request(
            "POST",
            "/oauth/renew_refresh_token/",
            form={
                "client_key": client_key,
                "refresh_token": refresh_token,
            },
            auth_required=False,
        )

    def userinfo(self, token: str, open_id: str) -> dict[str, Any]:
        """Get official user info for an authorized user."""
        return self.request(
            "GET",
            "/oauth/userinfo/",
            token=token,
            params={"open_id": open_id},
        )

    def comment_list(
        self,
        token: str,
        open_id: str,
        item_id: str,
        cursor: int,
        count: int,
    ) -> dict[str, Any]:
        """Get official comments for the authorized user's item."""
        return self.request(
            "GET",
            "/item/comment/list/",
            token=token,
            params={
                "open_id": open_id,
                "item_id": item_id,
                "cursor": str(cursor),
                "count": str(count),
            },
        )

    def comment_replies(
        self,
        token: str,
        open_id: str,
        item_id: str,
        comment_id: str,
        cursor: int,
        count: int,
    ) -> dict[str, Any]:
        """Get official replies for one comment."""
        return self.request(
            "GET",
            "/item/comment/reply/list/",
            token=token,
            params={
                "open_id": open_id,
                "item_id": item_id,
                "comment_id": comment_id,
                "cursor": str(cursor),
                "count": str(count),
            },
        )

    def reply_comment(
        self,
        token: str,
        open_id: str,
        item_id: str,
        content: str,
        comment_id: str | None = None,
    ) -> dict[str, Any]:
        """Reply to a comment on the authorized user's own item."""
        body = {
            "item_id": item_id,
            "content": content,
        }
        if comment_id:
            body["comment_id"] = comment_id
        return self.request(
            "POST",
            "/item/comment/reply/",
            token=token,
            params={"open_id": open_id},
            json_body=body,
        )

    def send_im_message(
        self,
        token: str,
        open_id: str,
        to_user_id: str,
        message_type: str,
        content: dict[str, Any],
        persona_id: str | None = None,
        client_msg_id: str | None = None,
    ) -> dict[str, Any]:
        """Send an enterprise IM message to a Douyin user."""
        body = {
            "to_user_id": to_user_id,
            "message_type": message_type,
            "content": json.dumps(content, ensure_ascii=False),
        }
        if persona_id:
            body["persona_id"] = persona_id
        if client_msg_id:
            body["client_msg_id"] = client_msg_id
        return self.request(
            "POST",
            "/enterprise/im/message/send/",
            token=token,
            params={"open_id": open_id},
            json_body=body,
        )

    def request(
        self,
        method: str,
        path: str,
        *,
        token: str | None = None,
        params: dict[str, str] | None = None,
        json_body: dict[str, Any] | list[Any] | None = None,
        form: dict[str, str] | None = None,
        headers: dict[str, str] | None = None,
        auth_required: bool = True,
    ) -> dict[str, Any]:
        """Call a Douyin official OpenAPI endpoint."""
        if auth_required and not token:
            raise DouyinOpenAPIError("调用 OpenAPI 需要 access-token 或 client-token")

        request_headers = {"content-type": "application/json"}
        if headers:
            request_headers.update(headers)
        if token:
            request_headers["access-token"] = token

        if form is not None:
            request_headers["content-type"] = "application/x-www-form-urlencoded"

        response = self._session.request(
            method.upper(),
            self._url(path),
            params=params,
            json=json_body,
            data=form,
            headers=request_headers,
            timeout=(10, 30),
        )
        if response.status_code >= 400:
            msg = f"OpenAPI HTTP 请求失败: {response.status_code} {response.text}"
            raise DouyinOpenAPIError(msg)

        try:
            data = response.json()
        except ValueError as exc:
            msg = f"OpenAPI 响应不是 JSON: {response.text}"
            raise DouyinOpenAPIError(msg) from exc

        if not isinstance(data, dict):
            msg = "OpenAPI 响应不是 JSON object"
            raise DouyinOpenAPIError(msg)
        return data

    def _url(self, path: str) -> str:
        if path.startswith(("http://", "https://")):
            return path
        return f"{self.base_url}/{path.lstrip('/')}"
