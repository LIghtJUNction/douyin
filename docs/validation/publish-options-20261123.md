# Publishing options: source review and validation status

## Change

The official OAuth path accepts a local JPG with `publish video --cover FILE.jpg`. It snapshots the selected bytes, records their size and SHA-256, uploads multipart `image` through the v1 image endpoint, and maps the returned `data.image.image_id` to the v1 create request's `custom_cover_image_url`. A missing image result leaves a durable `cover_uploading` stage and blocks resume and create. A complete receipt reuses its image ID without reopening the local JPG.

Explicit `--ai-generated` and `--short-title` requests are rejected before uploading because the checked sources do not establish a usable contract for those requested settings. Native `--cover` is also rejected. These errors describe the current CLI's evidence and implementation boundary; they do not claim that every platform interface lacks the capability. Preflight and receipts distinguish requested, supported and encoded settings from platform verification.

`auth creator-login --capabilities` is an offline inspection command. The current QR completion implementation does not collect all Creator publishing material and cannot activate a publishing session. The command and completion messages say so explicitly and identify the prerequisites for the existing import and official OAuth routes.

## Source and validation evidence, 2026-11-23

Base: `d70bd348bfab096ed7e876aefaf6c1a5978150fe` (`origin/main`, fetched for this task).

Observed on the development machine: Rust and Cargo 1.100.0. The change applied cleanly to an isolated worktree, `cargo fmt --all` completed successfully, and `git diff --check` passed.

**Compilation and regression tests have not run.** The required shared Cargo outer lock was held by another active validation batch. One immediate acquisition attempt and two bounded waits (120 seconds, then 300 seconds) failed before any test command started. The existing owner and its queued work were left running. No separate Cargo target was created.

The source is provided for review with this limitation. No candidate release binary was built or installed, no release was published, and no live account upload or create request was made during this task. Existing documentation's earlier test results are historical baseline evidence, not a passing result for this candidate.

## Required local checks when the shared lock is available

Hold `/var/tmp/project-z-cargo-target/.project-z-cargo-owner.lock` for the following Cargo batch, as required by the workspace instructions:

```bash
cargo test --locked --offline --lib publish
cargo test --locked --offline --lib openapi::tests
cargo test --locked --offline --lib creator_login
cargo test --locked --offline --test publish_cli --test publish_native_cli --test creator_login_cli
cargo fmt --all -- --check
git diff --check
```

The added tests use local files, isolated configuration and loopback HTTP. They cover the multipart field and returned image ID, the create body's actual custom-cover field, pending-cover failure without create, receipt consistency and resume, unsupported-option gates before credentials or media processing, and offline QR capability inspection. They do not exercise a real account or certify platform acceptance.

## Protocol sources

- [Official v1 image upload](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-image-text/image-upload), checked 2026-11-23: multipart `image`, OAuth `open_id` and `access-token`, successful `data.image.image_id`.
- [Official v1 create video](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/video-create), checked 2026-11-23: `custom_cover_image_url` takes that image ID; `cover_tsp` selects a video frame by seconds. No verified AI-declaration or independent short-title mapping was established from this page.
- Existing native source: `src/publish_native_client.rs` emits an empty `item.common.item_title` and uses the VOD result's `poster_uri`. An empty field is not sufficient evidence for a nonempty short-title contract.
- Existing QR material collection and completion: `src/creator_login.rs`, `src/creator_login_finish.rs`; the missing Creator fields are not fabricated or replaced with a readiness flag.
