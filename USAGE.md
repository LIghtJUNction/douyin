# douyin-cli 使用指南

## 1. 安装与升级

最低 Rust 版本为 1.88。确认本机工具链：

```bash
rustc --version
cargo --version
```

安装 crates.io 稳定版：

```bash
cargo install douyin-cli --locked
```

强制升级：

```bash
cargo install douyin-cli --locked --force
```

从当前源码安装：

```bash
cargo install --path . --locked
```

确认命令可用：

```bash
douyin --version
douyin --help
```

网页采集和评论签名需要 `node`：

```bash
node --version
```

如果只使用官方 OAuth、OpenAPI 或 MCP，可以不安装 Node.js。

## 2. 认证方式

### 2.1 如何选择

| 场景 | 认证方式 | 命令入口 |
| --- | --- | --- |
| 网页搜索、作品下载、评论抓取 | 浏览器 Cookie | `douyin auth cookie-*` |
| 官方用户信息、评论 API、企业号私信 | 官方 OAuth | `douyin auth login` |
| MCP 中的官方 OpenAPI 工具 | 官方 OAuth | `douyin mcp` |

两种认证相互独立，可以同时保存在同一个配置文件中。

### 2.2 Cookie 登录

1. 在浏览器登录 `douyin.com`。
2. 打开开发者工具的 Network/网络面板。
3. 选择一个发往 `douyin.com` 的请求。
4. 从 Request Headers/请求头复制完整 Cookie 值，不要复制 `Cookie:` 前缀。
5. 保存并检查：

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=...; 其他字段=..."
douyin auth cookie-status --offline
douyin auth cookie-status
```

Cookie 至少应包含可识别的 `sessionid` 或 `ttwid` 字段。`--offline` 不访问网络，只检查本地格式；普通 `cookie-status` 会请求登录态接口并区分“已登录”“未登录或已过期”和“因验证码、风控或上游变化而无法确认”。匿名网页接口返回成功不能证明 Cookie 有效。状态检查不会输出 Cookie 或响应正文。

环境变量方式：

```bash
export DOUYIN_COOKIE="完整 Cookie 字符串"
douyin auth cookie-login
douyin -u "关键词" -t search -l 5 --no-download
unset DOUYIN_COOKIE
```

删除已保存 Cookie：

```bash
douyin auth cookie-logout
```

真实 Cookie 属于账号凭据，不要粘贴到聊天、Issue、日志或 Git 仓库。

### 2.3 官方 OAuth 登录

OAuth 需要抖音开放平台应用。准备：

- `client_key`
- `client_secret`
- 应用允许的回调地址
- 所需 scope，例如 `user_info`、`item.comment` 或 `enterprise.im`

推荐本机回调方式。先在应用后台允许 `http://127.0.0.1:8787/callback`：

```bash
export DOUYIN_CLIENT_KEY="你的 client_key"
export DOUYIN_CLIENT_SECRET="你的 client_secret"

douyin auth login \
  --scope user_info \
  --scope item.comment \
  --listen \
  --callback-port 8787
```

CLI 默认输出授权链接和终端二维码。只输出链接时添加 `--no-qr`。

手动回调方式：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --scope item.comment
```

浏览器授权后，取回调 URL 中的 `code`：

```bash
douyin auth code --code "授权码"
```

授权维护：

```bash
douyin auth status
douyin auth status --json
douyin auth refresh
douyin auth logout
```

## 3. 网页采集与下载

网页采集使用已保存 Cookie。通用形式：

```text
douyin -u <目标> -t <类型> -l <数量> [其他选项]
```

常用例子：

```bash
# 搜索前 5 条，仅保存数据
douyin -u "搜索关键词" -t search -l 5 --no-download

# 下载单个作品
douyin -u "https://www.douyin.com/video/作品ID" -t aweme

# 下载账号主页前 20 个作品
douyin -u "https://www.douyin.com/user/用户ID" -t post -l 20

# 批量目标、指定目录、保存标题和封面
douyin -u targets.txt \
  -p ./downloads \
  --download-title \
  --download-cover
```

采集类型：

- `post`：账号发布作品
- `favorite`：账号喜欢作品
- `music`：音乐关联作品
- `hashtag`：话题关联作品
- `search`：关键词搜索
- `following`：关注列表
- `follower`：粉丝列表
- `collection`：收藏合集
- `mix`：作品合集
- `aweme`：单作品

搜索筛选：

```bash
douyin -u "关键词" -t search -l 20 \
  --sort-type 2 \
  --publish-time 7 \
  --filter-duration 1-5 \
  --no-download
```

- `--sort-type`：`0` 综合、`1` 最多点赞、`2` 最新
- `--publish-time`：`0` 不限、`1` 一天内、`7` 一周内、`180` 半年内
- `--filter-duration`：`0-1`、`1-5`、`5-10000`

## 4. 网页评论

抓取一级评论：

```bash
douyin comment "https://www.douyin.com/video/作品ID" --limit 100
```

同时抓取回复并写入文件：

```bash
douyin comment "https://www.douyin.com/video/作品ID" \
  --limit 100 \
  --with-replies \
  --reply-limit 50 \
  --format raw \
  --output comments.json
```

生成 ChatML 数据：

```bash
douyin comment "https://www.douyin.com/video/作品ID" \
  --with-replies \
  --format chatml-jsonl \
  --min-comment-digg 5 \
  --min-reply-digg 2 \
  --output comments.jsonl
```

输出格式：

- `raw`：标准评论 JSON
- `chatml-jsonl`：每行一条 ChatML 样本
- `chatml-json`：ChatML JSON 数组

## 5. 热词、热梗与需求发现

本地离线分析文件：

```bash
douyin insights comments.json --top 20 --min-count 2 --format json
douyin insights crawler.json -o insights.json
```

从 stdin 读取，并输出 Markdown：

```bash
cat comments.jsonl | douyin insights - --format markdown -o insights.md
```

输入格式包括：

- raw comments JSON，递归读取一级评论和 `replies`，并使用互动数字作为排序权重
- crawler JSON 数组或对象中的 `desc`、`text`、`tag` 与 `text_extra[].tag_name`
- ChatML JSON 或每行一条记录的 ChatML JSONL
- 每行一条文本记录的纯文本

JSON 顶层字段为 `input_count`、`hot_words`、`hot_memes`、`demands`。热词和热梗条目包含 `text`、`count`、`score`；需求条目还包含命中的 `signals`。分析使用停用词、短语切分、重复频次和意图关键词等确定性启发式规则，不访问网络，也不表示算法理解真实语义。

## 6. 作品表现离线统计

统计 crawler 输出中的作品元数据：

```bash
douyin stats search_陈震同学.json --author 陈震同学 --sort score --top 10
```

输入支持：

- crawler 扁平 JSON 数组
- 单个作品对象
- 对象中的 `items`、`aweme_list` 或 `data` 数组

可用排序为 `score`、`interactions`、`likes`、`comments`、`collects`、`shares`、`duration`、`latest`。使用 `--format markdown` 可输出包含总体汇总、字段覆盖、时长分桶、作者、话题和 Top 作品的 Markdown 报告。

`score` 在当前匹配集合内对点赞、评论、收藏、分享分别计算 `ln(1+x)/ln(1+max)`，再按 35%、20%、20%、25% 加权为 0–100 分。输入没有播放量，因此该分数不是互动率，也不应跨不同输入集合直接比较。

此命令只统计已采集的元数据，不分析 Hook、镜头、媒体画面、声音、字幕。

## 7. 官方 OpenAPI

完成 OAuth 后，可以省略重复的 token 和 `open_id` 参数：

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
douyin api comment-replies \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID"
```

回复评论：

```bash
douyin api comment-reply \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID" \
  --content "谢谢反馈"
```

写操作会要求确认；自动化调用可以显式添加 `--yes`。

企业号私信需要应用已开通 `enterprise.im`，并从事件回调取得 `to_user_id`：

```bash
douyin api im-message-send \
  --to-user-id "$DOUYIN_TO_USER_ID" \
  --text "你好，已收到" \
  --yes
```

通用同源 OpenAPI 请求：

```bash
douyin api request GET /oauth/userinfo/ \
  --param open_id="$DOUYIN_OPEN_ID"

douyin api request POST /item/comment/reply/ \
  --param open_id="$DOUYIN_OPEN_ID" \
  --json '{"item_id":"xxx","comment_id":"xxx","content":"谢谢反馈"}'
```

为避免 token 泄露，通用请求会拒绝指向其他域名的绝对 URL。

## 8. MCP

启动 stdio MCP 服务器：

```bash
douyin mcp
```

配置示例：

```json
{
  "mcpServers": {
    "douyin": {
      "command": "douyin",
      "args": ["mcp"]
    }
  }
}
```

命令行注册：

```bash
claude mcp add douyin -- douyin mcp
codex mcp add douyin -- douyin mcp
```

MCP 工具包括：

- `hot_words`（离线，只读，无需 OAuth）
- `hot_memes`（离线，只读，无需 OAuth）
- `demand_discovery`（离线，只读，无需 OAuth）
- `auth_status`
- `userinfo`
- `comment_list`
- `comment_replies`
- `comment_reply`
- `im_message_send`
- `openapi_request`

三个离线洞察工具的输入均为必填字符串数组 `texts`，并可传 `top`（默认 `20`）和 `min_count`（默认 `2`）。其余官方 OpenAPI 工具按各自要求使用 OAuth。

## 9. 配置、环境变量与退出

配置路径：

- Linux/macOS：`~/.config/douyin-cli/config/settings.json`
- Windows：`%APPDATA%\douyin-cli\config\settings.json`
- 自定义或测试隔离：`DOUYIN_HOME=/path/to/directory`

环境变量：

- `DOUYIN_COOKIE`
- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`
- `DOUYIN_HOME`

退出命令：

```bash
douyin auth cookie-logout
douyin auth logout
```

## 10. 故障排查

```bash
# 查看所有公开命令
douyin --help

# Cookie 仅做本地格式检查
douyin auth cookie-status --offline

# Cookie 联网确认网页登录态
douyin auth cookie-status

# OAuth 状态和机器可读输出
douyin auth status
douyin auth status --json

# 查看集成信息
douyin obscura manifest
douyin obscura status
```

若 `cookie-status` 报告“无法确认”，可先用 `--offline` 确认本地格式。验证码、风控或上游接口变化都可能阻止在线确认；匿名网页接口返回成功不能证明 Cookie 有效。检查过程不会输出 Cookie 或响应正文。遇到验证码或风控时，降低请求频率、重新从浏览器获取有效 Cookie，并避免高频并发请求。
