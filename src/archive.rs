//! Incremental post archives retain disappeared posts and refresh current ones.
use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};

pub fn save_posts(current: &mut Value, path: &Path) -> Result<(), String> {
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("json.lock"))
        .map_err(|_| "无法创建作品归档锁")?;
    lock.try_lock()
        .map_err(|_| "作品归档正被其他进程使用或无法加锁，请稍后重试")?;
    let result = (|| {
        merge_posts(current, path)?;
        crate::io::write_output(
            &serde_json::to_string_pretty(current).map_err(|_| "作品归档序列化失败")?,
            Some(path),
        )
    })();
    // Explicitly unlock: a concurrent process spawn can briefly inherit the
    // open file description before exec closes it, extending a drop-only lock.
    let unlocked = lock.unlock().map_err(|_| "无法释放作品归档锁".into());
    result.and(unlocked)
}

fn merge_posts(current: &mut Value, path: &Path) -> Result<(), String> {
    let previous = match std::fs::metadata(path) {
        Ok(_) => Some(
            serde_json::from_str::<Value>(&crate::io::read_input(
                path.to_str().ok_or("归档路径不是 UTF-8")?,
            )?)
            .map_err(|_| "旧作品归档不是合法 JSON，已保留原文件")?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("无法检查已有作品归档，已保留原文件".into()),
    };
    merge(current, previous.as_ref())
}

fn id(item: &Value) -> Result<String, String> {
    let value = item
        .get("aweme_id")
        .or_else(|| item.get("id"))
        .ok_or("归档作品缺少 ID，已保留原文件")?;
    let id = match value {
        Value::String(id) if !id.is_empty() => id.clone(),
        Value::Number(number) if number.as_u64().is_some() => number.to_string(),
        _ => return Err("归档作品 ID 无效，已保留原文件".into()),
    };
    if !id.bytes().all(|byte| byte.is_ascii_digit()) || id.bytes().all(|byte| byte == b'0') {
        return Err("归档作品 ID 必须是正整数，已保留原文件".into());
    }
    Ok(id.trim_start_matches('0').into())
}

fn merge(current: &mut Value, previous: Option<&Value>) -> Result<(), String> {
    if let Some(previous) = previous
        && previous.is_object()
        && previous.get("target").is_some()
        && previous.get("target") != current.get("target")
    {
        return Err("已有作品归档属于其他目标，已保留原文件".into());
    }
    let fresh = current["items"].as_array().ok_or("采集结果缺少作品列表")?;
    let old = match previous {
        None => &[][..],
        Some(Value::Array(items)) => items,
        Some(value) => value["items"]
            .as_array()
            .ok_or("旧作品归档缺少 items 数组，已保留原文件")?,
    };
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    let mut updated = 0usize;
    let fresh_ids = fresh.iter().map(id).collect::<Result<HashSet<_>, _>>()?;
    let mut old_seen = HashSet::new();
    for item in old {
        let old_id = id(item)?;
        if old_seen.insert(old_id.clone()) && fresh_ids.contains(&old_id) {
            updated += 1;
        }
    }
    for item in fresh.iter().chain(old) {
        let key = id(item)?;
        if seen.insert(key.clone()) {
            merged.push((key, item.clone()));
        }
    }
    // Douyin's decimal IDs increase over time. Compare without floating point
    // or u64 assumptions so large legacy IDs retain their exact ordering.
    merged.sort_by(|(left, _), (right, _)| {
        let left = left.trim_start_matches('0');
        let right = right.trim_start_matches('0');
        right.len().cmp(&left.len()).then_with(|| right.cmp(left))
    });
    let count = merged.len();
    current["incremental"] = json!({"enabled":true,"fetched":fresh.len(),"updated":updated,"archived":count,"new":fresh_ids.len()-updated,"retained":count-fresh_ids.len()});
    current["pagination_scope"] = json!("current_fetch");
    current["items"] = Value::Array(merged.into_iter().map(|(_, item)| item).collect());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn updates_statistics_retains_history_and_sorts_exact_large_ids() {
        let old = json!({"target":"account-a","items":[{"aweme_id":"9007199254740993","digg_count":1},{"aweme_id":"2"},{"aweme_id":"2"}]});
        let mut current = json!({"target":"account-a","has_more":true,"items":[{"aweme_id":"9007199254740993","digg_count":50},{"aweme_id":"9007199254740994"}]});
        merge(&mut current, Some(&old)).unwrap();
        assert_eq!(current["items"][0]["aweme_id"], "9007199254740994");
        assert_eq!(current["items"][1]["digg_count"], 50);
        assert_eq!(current["items"][2]["aweme_id"], "2");
        assert_eq!(
            current["incremental"],
            json!({"enabled":true,"fetched":2,"updated":1,"archived":3,"new":1,"retained":1})
        );
        assert_eq!(current["has_more"], true);
    }
    #[test]
    fn accepts_legacy_arrays_but_rejects_other_targets_and_malformed_archives() {
        let mut current = json!({"target":"a","items":[{"aweme_id":"3"}]});
        merge(&mut current, Some(&json!([{"id":2}]))).unwrap();
        assert_eq!(current["items"].as_array().unwrap().len(), 2);
        assert!(merge(&mut current, Some(&json!({"target":"b","items":[]}))).is_err());
        assert!(merge(&mut current, Some(&json!({}))).is_err());
        assert!(merge(&mut current, Some(&json!([{"title":"missing-id"}]))).is_err());
        for invalid in [json!({"id":"abc"}), json!({"id":0}), json!({"id":"-1"})] {
            assert!(merge(&mut current, Some(&json!([invalid]))).is_err());
        }
    }
    #[test]
    fn invalid_existing_archive_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("posts.json");
        std::fs::write(&path, "damaged").unwrap();
        let mut current = json!({"items":[{"aweme_id":"1"}]});
        assert!(merge_posts(&mut current, &path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "damaged");
    }
    #[test]
    fn competing_writers_cannot_overwrite_a_locked_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("posts.json");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("json.lock"))
            .unwrap();
        lock.lock().unwrap();
        let mut current = json!({"target":"a","items":[{"aweme_id":"1"}]});
        assert!(save_posts(&mut current, &path).is_err());
        assert!(!path.exists());
        lock.unlock().unwrap();
        drop(lock);
        save_posts(&mut current, &path).unwrap();
        let mut next = json!({"target":"a","items":[{"aweme_id":"2"}]});
        save_posts(&mut next, &path).unwrap();
        assert_eq!(next["items"].as_array().unwrap().len(), 2);
    }
}
