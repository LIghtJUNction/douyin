use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::net::{integer, truthy};

pub struct Page {
    pub items: Vec<Value>,
    pub cursor: String,
    pub has_more: bool,
}

/// Collect only real progress. Repeated cursors and pages are errors rather
/// than silent truncation or an unbounded loop. limit=0 remains unlimited.
/// The fetch callback receives the remaining unique-item budget (None when unlimited).
pub fn collect(
    mut cursor: String,
    limit: usize,
    id_key: &str,
    mut fetch: impl FnMut(&str, Option<usize>) -> Result<Page, String>,
) -> Result<Value, String> {
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    cursors.insert(cursor.clone());
    let mut pages = 0;
    let mut empty_pages = 0;
    loop {
        let remaining = (limit > 0).then(|| limit - items.len());
        let page = fetch(&cursor, remaining)?;
        pages += 1;
        let before = items.len();
        let page_length = page.items.len();
        for (index, item) in page.items.into_iter().enumerate() {
            let id = item[id_key]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| item[id_key].as_u64().map(|v| v.to_string()))
                .ok_or("分页记录缺少稳定 ID")?;
            if seen.insert(id) {
                items.push(item);
            }
            if limit > 0 && items.len() >= limit {
                let truncated = index + 1 < page_length;
                return Ok(json!({"items":items,"has_more":page.has_more || truncated,
                        "cursor":if truncated{cursor}else{page.cursor.clone()},"next_cursor":page.cursor,
                        "page_truncated":truncated,"pages":pages,"limit_reached":true}));
            }
        }
        if !page.has_more {
            return Ok(
                json!({"items":items,"has_more":false,"cursor":page.cursor,"pages":pages,"limit_reached":false}),
            );
        }
        if !cursors.insert(page.cursor.clone()) {
            return Err("接口分页游标重复，停止以避免重复请求和数据截断".into());
        }
        if items.len() == before {
            empty_pages += 1;
            if empty_pages >= 10 {
                return Err("接口连续 10 页没有新增记录，停止以避免无限翻页".into());
            }
        } else {
            empty_pages = 0;
        }
        cursor = page.cursor;
    }
}

pub fn parse_page(value: &Value, list_key: &str, cursor_key: &str) -> Result<Page, String> {
    let has_more = value.get("has_more").ok_or("接口分页缺少 has_more")?;
    let has_more = match has_more {
        Value::Bool(_) => truthy(has_more),
        other if integer(other).is_some() => truthy(other),
        _ => return Err("接口分页标记无效".into()),
    };
    let items = match value.get(list_key) {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) if !has_more => Vec::new(),
        _ => return Err("接口未返回预期的数据列表".into()),
    };
    let cursor = value[cursor_key]
        .as_str()
        .map(str::to_owned)
        .or_else(|| integer(&value[cursor_key]).map(|n| n.to_string()));
    if has_more && cursor.is_none() {
        return Err("接口声称有下一页但未返回游标".into());
    }
    Ok(Page {
        items,
        cursor: cursor.unwrap_or_else(|| "0".into()),
        has_more,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deduplicates_boundary_items_and_honors_exact_limit() {
        let result = collect("0".into(), 3, "aweme_id", |cursor, remaining| {
            assert_eq!(remaining, Some(if cursor == "0" { 3 } else { 1 }));
            Ok(if cursor == "0" {
                Page {
                    items: vec![json!({"aweme_id":"1"}), json!({"aweme_id":"2"})],
                    cursor: "2".into(),
                    has_more: true,
                }
            } else {
                Page {
                    items: vec![
                        json!({"aweme_id":"2"}),
                        json!({"aweme_id":"3"}),
                        json!({"aweme_id":"4"}),
                    ],
                    cursor: "4".into(),
                    has_more: false,
                }
            })
        })
        .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 3);
        assert_eq!(result["items"][2]["aweme_id"], "3");
        assert_eq!(result["has_more"], true);
        assert_eq!(result["page_truncated"], true);
        assert_eq!(result["cursor"], "2");
    }
    #[test]
    fn refuses_nonprogress_in_unlimited_runs() {
        assert!(
            collect("0".into(), 0, "id", |_, remaining| {
                assert_eq!(remaining, None);
                Ok(Page {
                    items: vec![json!({"id":"1"})],
                    cursor: "0".into(),
                    has_more: true,
                })
            })
            .is_err()
        );
        assert!(
            parse_page(
                &json!({"aweme_list":[],"has_more":true}),
                "aweme_list",
                "max_cursor"
            )
            .is_err()
        );
        assert!(parse_page(&json!({"status_code":0}), "aweme_list", "max_cursor").is_err());
    }

    #[test]
    fn empty_pages_may_advance_to_a_nonempty_page_within_bounds() {
        let result = collect("0".into(), 1, "aweme_id", |cursor, _| {
            Ok(if cursor == "0" {
                Page {
                    items: vec![],
                    cursor: "1".into(),
                    has_more: true,
                }
            } else {
                Page {
                    items: vec![json!({"aweme_id":"1"})],
                    cursor: "2".into(),
                    has_more: false,
                }
            })
        })
        .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn comments_on_the_same_video_keep_their_own_identity() {
        let result = collect("0".into(), 0, "cid", |_, _| {
            Ok(Page {
                items: vec![
                    json!({"cid":"1","aweme_id":"v"}),
                    json!({"cid":"2","aweme_id":"v"}),
                ],
                cursor: "2".into(),
                has_more: false,
            })
        })
        .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
    }
}
