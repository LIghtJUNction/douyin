---
name: douyin
description: Develop, install, test, package, troubleshoot, or operate this Rust douyin CLI, including Android identity, Cookie web crawls, OAuth/OpenAPI, comments, downloads, MCP, and livestreaming.
---

# Douyin Rust CLI

Treat this repository as a Rust CLI. `Cargo.toml` and committed `Cargo.lock` are the build source; do not reintroduce Python packaging. The small Android PackageManager helper in `assets/device-metadata/` is Java source with a bundled DEX; it reads public installed package metadata and, for the pinned supported APK build, invokes its pure Cronet default User-Agent generator in a temporary VM. The generator cache is initialized from PackageManager in that VM only. This value is private candidate metadata, not verified final wire User-Agent. After Token-only account verification it can be saved as the core native Token request agent; other hosts retain the legacy session agent. The helper runs temporarily via ADB and is cleaned up. Ordinary CLI builds and users do not need Java or an Android SDK.

## Discover Before Acting

Use current Clap help rather than memory:

```bash
git status -sb
cargo run --locked --offline -- --help
cargo run --locked --offline -- auth --help
cargo run --locked --offline -- api --help
cargo run --locked --offline -- comment --help
```

If docs and help disagree, inspect `src/main.rs` and the owning Rust module, then update the docs. Keep every public command visible in applicable help. `docs/rebuild.md` distinguishes implemented code, local tests, and real-device acceptance; retain that distinction.

## Install or Upgrade

```bash
cargo install douyin-cli --locked
cargo install douyin-cli --locked --force
cargo install --path . --locked
douyin --version
```

Require Rust 1.100+ using `rust-toolchain.toml`. The deliberately deleted implementation must not be restored. The webpage Cookie backend and signer are still being rebuilt; do not claim they work because Cookie configuration commands exist.

## Route Authentication

Use `douyin auth --adb` for the current native crawl/comment clients. Import actual login state and corroborated device/install IDs from the selected rooted Android phone. The supported Token snapshot also carries its nonempty server-issued token_tlb_tag; treat it as sensitive and require Token/tag stability together. Older saved sessions without this optional field remain valid. The nonempty-tag branch passed isolated real-device import and direct auth status with ADB shadowed by a failing stub; no ADB invocation occurred. An earlier cold-start import rejected an unsupported Provider row shape; the cause is still unresolved, so do not claim startup reliability. Live permissions with the new nonempty-tag branch remain pending. Do not persist track_data or signing keys. Native login (`session.json`), website Cookie (`cookie.json`), and official OAuth (`oauth.json`) are separate. Use OAuth for `douyin api` and its MCP tools; `native_me` and `native_live_permissions` use native login and offline tools need no auth. Media downloads must not forward authentication credentials to CDN hosts.

```bash
douyin device
douyin auth --adb
douyin auth --adb --serial "$ANDROID_SERIAL"
douyin auth status
douyin auth status --offline
```

The user's rebuild request authorizes read-only real-device/API acceptance. Keep live social writes separate from fixture tests. Do not report risk-controlled empty responses as successful complete crawls.

Respect any temporary restriction on device use in the conversation. Offline protocol and script tests must not access ADB or the phone.

Native transport selection is `DOUYIN_NATIVE_TRANSPORT=direct|phone`, default direct. Never silently use a connected phone: the user requires independent operation after import, including livestreaming. The phone transport is only a read-only protocol research tool. It checks effective cgroup freezing before installing its support service; ask for unlock/foreground only when current evidence requires it. Never write freezer controls or force-stop the app to bypass this check. It opens the app and uses its GET provider; it checks live account, device/install IDs and app versions on every call. Its official Frida SDK worker is isolated behind a size/sequence/deadline checked subprocess protocol. Do not export private signing material casually or expose it in logs.

Livestreaming must use the imported Android identity and Android room lifecycle, with evidence the service treats it as Android broadcasting. OBS integration must support both OBS pushing to Douyin and Douyin live video as an OBS input. Configuring an RTMP target alone does not prove mobile broadcasting. Do not claim completion from OBS-only tests.

`guard::ReeSigner` is a verified software signing primitive, not the complete HTTP signer. The current SDK subprocess snapshot uses private scalar hex and base64 uncompressed public point; validate the key pair and reject incomplete/NULL snapshots. A real SDK test verified a native P-256/SHA-256 DER signature locally. Account/token/key binding, canonical request payloads, guard headers, refresh and server acceptance remain unfinished. The developer guard-probe example accepts sensitive data only on bounded stdin and prints booleans; never persist real device fixtures or private keys in source/tests/logs.

The SDK ticket/path/timestamp request message is literal concatenation, not form encoding. ECDSA and `guard::HmacSigner` request probes have both verified real SDK signatures/MACs for fixed public fixture messages. HMAC uses SHA-256 and a base64 key snapshot. These comparisons do not establish HTTP header mode selection, live timestamp derivation, account binding or server acceptance. Never automatically downgrade a required TEE/attestation mode to REE/HMAC.

The successful 17.26.2 prototype does not prove the newly integrated Rust/17.9.5 pipeline. Current Rust CLI phone-mode me has passed on the real device, but subsequent initialization can fail during script creation before a search request is sent; stable full CLI acceptance remains pending. Source builds now need a C compiler, libclang and the Frida core devkit; preserve bundled Java bridge licensing and pinned asset digests.

Keep API `version_code`, APK `manifest_version_code`, and `update_version_code` separate. Read them from installed package metadata and cross-check the APK version; never infer one from another. A transient successful signer experiment is not proof that the production backend works.

Never ask the user to paste a real Cookie, client secret, or token into chat; keep secrets and response bodies out of logs and repositories.

### Native Token

For the inspected 40.8.0 / 400800 / 400801 / 40809900 build, `auth --adb` also imports the native Token with SDK protocol header 2 and Passport SDK header 60760, corroborated from the APK header factory. It normally opens the app to initialize SDK state, requires a stable Token, bootstraps the account from the Token-only self/profile endpoint and independently checks that account before atomic storage. Install ID comes from applog_stats.xml corroborated in current Keva, not WebView Cookie. Missing/expired/conflicting Cookie snapshots do not block this supported native login. Optional compatibility Cookies are retained only after matching the install ID and separately verifying the same account through Passport; the final Token snapshot is checked again before save. Device/install identity is reread from XML/Keva before this final Token check; reject changed identifiers even when older Keva values remain present, preserving the existing config. Summaries report nativeCookiesImported separately. A Token-only Passport bootstrap returned HTTP 403 in acceptance; do not substitute it for the verified self/profile bootstrap or retry an HTTP rejection as a transport failure. Runtime core requests and status prefer this validated Token and do not require Cookie or ADB. Unsupported builds retain the older Cookie path. General Token headers remain scoped to aweme.snssdk.com. The separately verified fixed GET webcast.amemv.com/webcast/user/permission/ uses its own narrow credential selection; this does not authorize other webcast paths or any media/OpenAPI host. `live permissions` first checks the current account and uses its returned sec_uid, then returns a summary excluding last_room and arbitrary credential values. It requires direct transport and imported native Token, with no Cookie fallback. MCP exposes native_live_permissions through the same code. Summaries expose import/verification booleans, never credential values. This establishes basic account authentication, not complete HTTP signing, mobile broadcast classification or full endpoint acceptance.

Direct GET requests to aweme.snssdk.com, including Token account verification retries, add a freshly generated millisecond `x-ss-req-ticket`. This is a verified transport field, not a signature. Do not truncate it to seconds or extend it to the fixed Webcast permission endpoint without separate evidence.

For optional or legacy Cookie import, when multiple supported databases exist, importing those Cookies requires identical live native identity/session fields and retains only rows present unchanged in every snapshot. Conflicting tracker/device/region rows are omitted; identity mismatches or inconsistent identity-row attributes reject import. Keep each database/WAL snapshot independently coherent and never pick conflicting values by path priority.

### Cookie

```bash
douyin auth cookie-login --cookie-file ./private-cookie.txt
douyin auth cookie-status --offline
douyin auth cookie-status
douyin auth cookie-logout
```

`--offline` validates local format only. Online status uses Douyin's login-state endpoint and may be unable to confirm under captcha, risk control, or an unrecognized response. An anonymous endpoint succeeding is not authentication proof.

Cookie auth accepts `--cookie`, `--cookie-file` (including stdin `-`), or `DOUYIN_COOKIE`. The crawl and comment Cookie backends are not yet connected; consult current help instead of assuming old flags exist.

### OAuth

```bash
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --scope user_info --listen --callback-port 8787
douyin auth status --oauth
douyin auth refresh
douyin auth logout
```

Ensure the platform app allows the callback URL. Use `douyin auth code --code <code>` for a manual callback.

`guard::TokenProofs` parses the token-associated combined ts_sign / ts_sign_ree
fields. A legacy opaque proof belongs only to TEE; never reuse it as REE.
`require_ree` rejects missing software proof and does not choose a signing mode.
The public six-case fixture verifies the installed model's annotations/getters,
not the app's live Gson parser or server acceptance; see assets/guard-fixtures.

The guard-probe example also accepts an explicit `request` object on bounded
stdin (ticket/path/timestamp), retaining booleans-only output. An authorized
real native following GET returned status 0; the final Java request headers'
req_sign_ree verified locally with the exported HMAC key and that request's
actual token/path/timestamp. ECDSA did not verify this HMAC sample. This proves
request-context crypto compatibility, not complete independent HTTP auth.
TTTokenManager.addRequestHeader alone omitted client-data in that experiment;
the later network request processing supplied it. Never infer final headers
from the early factory alone. No real header values or private fixtures were
persisted. A later search/replay probe failed at RPC, so host replay remains
unverified.

Later search probes returned three results natively and verified req_sign_ree
with local HMAC, while host replay returned a risk-controlled empty list even
with the live CookieHandler Cookie and unchanged client-data wire bytes.
frameSign returns structured frame fields, not an HTTP-header map. The actual
Java request chain reaches X.1T5e/SsCronetHttpClient and CronetHttpURLConnection;
connection setup continues after Request.getHeaders (including UA handling
and possible URL dispatch). The current helper's app_process http.agent is
not verified as the final app TTNet UA. Do not claim the entire outgoing
context has been captured or complete signed search/live replay works. Basic Token-only core authentication has since been verified separately. See docs/rebuild.md
for exact evidence and failed probe limitations.

## Web Workflows

```bash
douyin -u "关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin -u "https://www.douyin.com/user/..." -t post -l 20
douyin comment "https://www.douyin.com/video/..." --limit 100 --with-replies --format chatml-jsonl --output comments.jsonl
```

Root crawl types: `post`, `favorite`, `music`, `hashtag`, `search`, `following`, `follower`, `collection`, `mix`, `aweme`, and `feed`. A listed command is not proof of endpoint acceptance. Posts merge existing archives under a file lock; pagination and limits describe the current fetch, not the retained archive.

Avoid unrequested live requests outside the authorized rebuild/acceptance scope.

## OpenAPI and MCP

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
douyin api request GET /oauth/userinfo/ --param open_id="$DOUYIN_OPEN_ID"
douyin mcp
```

Only send same-origin OpenAPI paths. Treat write operations as confirmation-gated unless the user explicitly supplies `--yes`.

Use `DOUYIN_HOME` to isolate auth/config state during tests:

```bash
DOUYIN_HOME=/tmp/douyin-command-check cargo run --locked --offline -- auth --help
```

## Verify Changes

After Rust, dependency, CLI, docs, or packaging changes, run:

```bash
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --release --locked --offline
cargo package --locked --offline --allow-dirty
cargo run --locked --offline -- --help
```

Ensure `src/cookie.rs` appears in `cargo package --list`; the root `/cookie.*` ignore rule must never match Rust source files.

For phone agent changes, run `node tools/test-phone-agent.mjs` on the development machine. Node is not a runtime requirement. Package all `assets/phone/` source, bridge, digest catalog, origin and license files.

Also ensure the package contains `assets/device-metadata/DeviceMetadata.java`, `assets/device-metadata/classes.dex`, and `tools/build-device-helper.sh`. After changing helper source, regenerate its DEX with JDK, Android platform 36 and build-tools 35.0.0, then check reproducibility:

```bash
bash tools/build-device-helper.sh
bash tools/build-device-helper.sh --check
```

### Binder crypto fixture probe

`cargo run --locked --offline --example guard-adb-probe -- [serial]` explicitly checks a public fixture against the installed TicketGuardProvider without Frida. It checks main-user selection, the inspected APK version and snapshot stability; only booleans leave the module. The provider logs its URI, so never replace fixture content with a real ticket or private request. Native HMAC agreement and REE key-pair validity do not prove account binding or independent HTTP authentication. This is not a runtime fallback or an auth import path.

`cargo run --locked --offline --example guard-adb-probe -- --token-context [serial]` separately compares the Token provider's `track_data.reePub` with the validated software key pair, requiring both snapshots to remain unchanged across the check. It rejects logged-out or ambiguous cursor rows and duplicate JSON fields. This confirms SDK snapshot consistency only; device keys may be shared across accounts, so account binding and independent-auth flags remain false. No real token/key material is persisted.

Native search metrics have shown final URL dispatch from aweme.snssdk.com to search3-search.amemv.com. Initial Java request headers/URL are not the final wire context; switching the independent Cookie request's host alone still returned a risk-controlled empty result. Do not hardcode this host or promote the candidate UA on that evidence. New SIGSEGV records appeared during the diagnostic period; cause remains unproven. Restore explicit research hooks and release retained Java wrappers before unloading; a syntax-checked cleanup change is not real-device crash acceptance.


### Android create-form constraints

The inspected VIDEO enum uses logStreamingType=general, sceneName=video, background=false and camera=true; THIRD_PARTY uses thirdparty / third_party, background=true and camera=false. These are SDK enum metadata, not proven wire constants or room classification. The VIDEO create-form branch does not itself set third_party, live_audio or screen_shot. Its optional external-camera business payload requires both the SDK setting and a connected external-camera service; do not select it merely because the media originates in OBS. Three later putAll sources can replace prior fields, so any mobile-form validation must inspect the final merged form, not an early builder. No create/continue/end requests have been authorized for read-only acceptance.

PermissionResult.broadcastConfig is scheduling configuration (scheduled_* fields), not a can-broadcast permission. `live permissions` exposes verified SDK scalar flag fields, including signed int32 sentinels, and known schedule fields; it excludes historical rooms, arbitrary added values and out-of-range flag numbers. Do not turn authentication_type, a settings flag, or status_code=0 into a mobile-broadcast acceptance claim.


`live info` summaries expose sdk_stream_type using inspected 40.8.0 Room.getStreamType precedence: room_layout=1 or layout=ROOM_TYPE_OFFICIAL_ACTIVITY (initialized to 1) -> media, then live_type_screenshot, live_type_third_party, live_type_audio, otherwise video. Raw response classification requires the needed fields with supported types; absent metadata must not default to video. This client SDK classification alone never proves Android broadcaster identity. createRoomWithPb returns ExtraResponse<Room,RoomCreateBlockExtra>; account/stream/room validation and blocker handling remain required before any real creation can be accepted. Never print/persist blocker tickets, unique IDs or raw challenge payloads in public summaries.

## Metasec configuration fixtures

`metasec::decode_constant` handles public APK constants; `Configuration` only encodes the pinned u2 model. Neither initializes the SDK or signs requests. Preserve all 17 positions, Java trim semantics, integer strings and observed map iteration order. Do not reconstruct Java HashMap order by sorting. The fake config fixture and its exact configured JSON text were produced by the real APK Java serializer in an isolated app_process, with s4 replaced only to avoid native initialization. The helper pins the APK hash and never operates on live account objects. See assets/metasec-fixtures/README.md. These fixtures do not establish runtime context readiness or server acceptance.

`metasec::digest`, `speck` and `helios` implement recovered arithmetic against public synthetic vectors and the published Speck standard vector. `metasec::field13` implements all three request-digest branches against public golden vectors. `metasec::gorgon`, `protobuf` and `envelope` cover additional wire primitives, but SDK words and application constants must come from the pinned target rather than other apps. Medusa omits later SDK state bits; Helios requires genuine caller-supplied context and secret material. Field 13 alone is not a complete Medusa header. Do not treat these primitives as complete headers, initialize SDK readiness artificially, or enable them in production requests before full context recovery and server acceptance.
