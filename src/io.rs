use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub fn read_input(path: &str) -> Result<String, String> {
    const MAX: u64 = 64 * 1024 * 1024;
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(fs::File::open(path).map_err(|_| format!("无法读取输入文件 {path}"))?)
    };
    let mut text = String::new();
    reader
        .take(MAX + 1)
        .read_to_string(&mut text)
        .map_err(|_| "输入不是可读取的 UTF-8 文本")?;
    if text.len() as u64 > MAX {
        return Err("输入超过 64 MiB 限制，请拆分文件".into());
    }
    Ok(text)
}

pub fn json_documents(input: &str) -> Result<Vec<serde_json::Value>, String> {
    if let Ok(value) = serde_json::from_str(input) {
        return Ok(vec![value]);
    }
    let documents: Result<Vec<_>, _> = input
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|_| format!("第 {} 行不是有效 JSON", index + 1))
        })
        .collect();
    let documents = documents?;
    if documents.is_empty() {
        return Err("输入没有数据".into());
    }
    Ok(documents)
}

pub fn write_output(text: &str, output: Option<&Path>) -> Result<(), String> {
    if let Some(path) = output {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut pending =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建输出临时文件")?;
        pending
            .write_all(text.as_bytes())
            .and_then(|_| pending.write_all(b"\n"))
            .and_then(|_| pending.as_file().sync_all())
            .map_err(|_| "写入输出文件失败")?;
        pending.persist(path).map_err(|_| "无法提交输出文件")?;
        Ok(())
    } else {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{text}").map_err(|_| "写入标准输出失败".into())
    }
}
