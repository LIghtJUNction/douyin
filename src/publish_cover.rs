//! A private JPEG snapshot ties the uploaded bytes to the saved cover receipt.
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

// Conservative decimal-byte interpretation of the documented 300 MB limit.
const MAX_COVER_BYTES: u64 = 300_000_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoverReceipt {
    pub bytes: u64,
    pub sha256: String,
    pub image_id: Option<String>,
}

impl CoverReceipt {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(5..=MAX_COVER_BYTES).contains(&self.bytes)
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || self
                .image_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty() || id.len() > 16 * 1024)
        {
            return Err("封面上传记录无效".into());
        }
        Ok(())
    }
}

pub(crate) struct CoverSnapshot {
    file: tempfile::NamedTempFile,
    pub receipt: CoverReceipt,
}

impl CoverSnapshot {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let mut source = File::open(path).map_err(|_| "无法读取本地 JPG 封面")?;
        let metadata = source.metadata().map_err(|_| "无法读取封面文件信息")?;
        if !metadata.is_file() || !(5..=MAX_COVER_BYTES).contains(&metadata.len()) {
            return Err("封面必须是非空普通 JPG 文件，本地上限为 300000000 字节".into());
        }
        let mut file = tempfile::Builder::new()
            .prefix("douyin-cover-")
            .suffix(".jpg")
            .tempfile()
            .map_err(|_| "无法创建封面私有快照")?;
        let mut context = Context::new(&SHA256);
        let mut buffer = [0; 64 * 1024];
        let mut bytes = 0;
        let mut first = Vec::with_capacity(3);
        let mut tail = [0; 2];
        loop {
            let count = source.read(&mut buffer).map_err(|_| "读取 JPG 封面失败")?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > MAX_COVER_BYTES {
                return Err("封面超过本地 300000000 字节限制".into());
            }
            first.extend(buffer[..count].iter().take(3 - first.len()).copied());
            tail = if count >= 2 {
                [buffer[count - 2], buffer[count - 1]]
            } else {
                [tail[1], buffer[0]]
            };
            context.update(&buffer[..count]);
            file.write_all(&buffer[..count])
                .map_err(|_| "无法写入封面快照")?;
        }
        if bytes != metadata.len() {
            return Err("封面在读取过程中发生变化，已停止上传".into());
        }
        // This is a marker check, not a claim of complete JPEG decoding.
        if first != [0xff, 0xd8, 0xff] || tail != [0xff, 0xd9] {
            return Err("封面必须具有完整的 JPEG 起止标记；不接受 PNG 或任意 URL".into());
        }
        file.as_file().sync_all().map_err(|_| "无法保存封面快照")?;
        let sha256 = context
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(Self {
            file,
            receipt: CoverReceipt {
                bytes,
                sha256,
                image_id: None,
            },
        })
    }

    pub(crate) fn reader(&self) -> Result<File, String> {
        self.file.reopen().map_err(|_| "无法读取封面快照")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_preserves_selected_bytes_after_source_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.jpg");
        let jpeg = b"\xff\xd8\xfffixture-only\xff\xd9";
        std::fs::write(&path, jpeg).unwrap();
        let cover = CoverSnapshot::read(&path).unwrap();
        std::fs::write(&path, b"changed").unwrap();
        let mut actual = Vec::new();
        cover.reader().unwrap().read_to_end(&mut actual).unwrap();
        assert_eq!(actual, jpeg);
        assert_eq!(cover.receipt.bytes, jpeg.len() as u64);
        cover.receipt.validate().unwrap();
        assert!(CoverSnapshot::read(&path).is_err());
    }

    #[test]
    fn invalid_jpeg_and_oversized_file_are_rejected_before_upload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.jpg");
        for bytes in [b"".as_slice(), b"\x89PNGfixture", b"\xff\xd8\xfftruncated"] {
            std::fs::write(&path, bytes).unwrap();
            assert!(CoverSnapshot::read(&path).is_err());
        }
        File::create(&path)
            .unwrap()
            .set_len(MAX_COVER_BYTES + 1)
            .unwrap();
        assert!(CoverSnapshot::read(&path).is_err());
    }
}
