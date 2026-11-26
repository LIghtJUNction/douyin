# Publishing options: source review and validation status

## Change

The official OAuth path accepts a local JPG with `publish video --cover FILE.jpg`. It snapshots the selected bytes, records their size and SHA-256, uploads multipart `image` through the v1 image endpoint, and maps the returned `data.image.image_id` to the v1 create request's `custom_cover_image_url`. A missing image result leaves a durable `cover_uploading` stage and blocks resume and create. A complete receipt reuses its image ID without reopening the local JPG.

Explicit `--ai-generated` and `--short-title` requests are rejected before uploading because the checked sources do not establish a usable contract for those requested settings. Native `--cover` is also rejected. These errors describe the current CLI's evidence and implementation boundary; they do not claim that every platform interface lacks the capability. Preflight and receipts distinguish requested, supported and encoded settings from platform verification.

`auth creator-login --capabilities` is an offline inspection command. The current QR completion implementation does not collect all Creator publishing material and cannot activate a publishing session. The command and completion messages say so explicitly and identify the prerequisites for the existing import and official OAuth routes.

## Completed local validation, 2026-11-24 (Asia/Taipei)

The candidate now passes the targeted library and CLI regression tests, formatting, diff validation, and release build. The exact release binary was installed atomically and read back successfully. These are local implementation results; no real OAuth upload or create request was made by this validation.

The reviewed feature candidate is `484d672ee3f36173690baf8e129fe4fc0c2cf003`, based on `d70bd348bfab096ed7e876aefaf6c1a5978150fe`. Actual compilation exposed a return-type error in `CoverSnapshot::reader`: its `Result<File, String>` returned an `&str` error. The correction converts that error to an owned string without changing snapshot or upload behavior.

| Command or gate | Actual result |
| --- | --- |
| `cargo test --locked --offline --lib publish` | 51 passed, 0 failed |
| `cargo test --locked --offline --lib openapi::tests` | 10 passed, 0 failed |
| `cargo test --locked --offline --lib creator_login` | 22 passed, 0 failed |
| `cargo test --locked --offline --test creator_login_cli` | 5 passed, 0 failed |
| `cargo test --locked --offline --test publish_cli` | 7 passed, 0 failed |
| `cargo test --locked --offline --test publish_native_cli` | 10 passed, 0 failed |
| `cargo fmt --all -- --check` | Passed after the requested line wrap |
| `git diff --check` | Passed |
| `cargo build --locked --offline --release --bin douyin` | Passed |
| Release `--version` and offline `auth creator-login --capabilities` | Passed; capabilities report no network access and no QR publishing activation |

The three CLI suites ran together in one Cargo command. Library name filters overlap, so their counts are not presented as a unique aggregate.

All Cargo validation used the existing shared target and held its outer owner lock across each batch. Earlier lock waits are historical, not passing evidence. On continuation, the USTC replacement cache lacked packages already present in the local crates.io cache. Twenty-six missing index entries and 43 checksum-verified crate archives were reused locally without a dependency download or global source-configuration change. The first real compilation found the type error above.

The first integration compilation was deliberately interrupted with SIGINT when video rendering and the workstation were under memory/swap pressure. Its outcome is recorded as interrupted, not passed. The subsequent batch used one Cargo build job and nice level 15; the integration suites completed. After the exact rustfmt line wrap, formatting and release gates completed in the same low-concurrency configuration.

## Installed artifact and scope

- Version: `douyin-cli 2026.11.20`
- Installed SHA-256: `22f7e0584006b3560a5fd5c7e1bbfa66895bea2be88090c236c1614ba4d8b872`
- Previous installed SHA-256, retained as rollback: `3439f49925950d14ee2bbd1616c5572f96df02442cbe850ecb1239225706d6b3`
- Final Rust source-file SHA-256 for `src/publish_cover.rs`: `5b9cf17530c3a94baa9c502da3c5e1d8b685af2bd169d8656e044f2cfd5e0849`

The release was copied to a task-owned candidate snapshot while the shared build lock was still held. Installation verified the candidate and previous binary, preserved a rollback copy, staged the new executable beside the target, used an atomic replacement, and verified the installed hash and version. No binary, build cache, or account material is committed to Git.

The task workspace's `geocore-city/cli-validation` directory contains the original failed/interrupted receipts, passing continuation receipts, logs, and installation receipt. Actual Creator UI publication belongs to its separate single-attempt journal; this validation does not certify publication, review status, AI labeling, or platform acceptance of a cover.

The tests use local files, isolated configuration and loopback HTTP. They verify the multipart image field, returned image ID, actual custom-cover create field, incomplete-cover blocking without create, receipt consistency and resume, unsupported-option gates before credentials or media processing, and offline QR capability inspection. Live OAuth behavior remains unverified.

## Protocol sources

- [Official v1 image upload](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-image-text/image-upload), checked 2026-11-23: multipart `image`, OAuth `open_id` and `access-token`, successful `data.image.image_id`.
- [Official v1 create video](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/video-create), checked 2026-11-23: `custom_cover_image_url` takes that image ID; `cover_tsp` selects a video frame by seconds. No verified AI-declaration or independent short-title mapping was established from this page.
- Existing native source: `src/publish_native_client.rs` emits an empty `item.common.item_title` and uses the VOD result's `poster_uri`. An empty field is not sufficient evidence for a nonempty short-title contract.
- Existing QR material collection and completion: `src/creator_login.rs`, `src/creator_login_finish.rs`; the missing Creator fields are not fabricated or replaced with a readiness flag.
