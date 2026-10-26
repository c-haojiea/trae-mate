// 文件读写工具:原子替换 + 容错加载 + 时间/脱敏辅助。移植自参考项目 fs_utils.rs。

use std::fs;
use std::io::Write;
use std::path::Path;

/// 读取 JSON,文件不存在返回默认值;解析失败时先备份损坏文件,再返回默认值。
pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    match fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(_) => {
                let _ = backup_corrupt(path);
                T::default()
            }
        },
        _ => T::default(),
    }
}

/// 把损坏文件复制为 `<原名>.corrupt_<unix_ms>`;只拷贝,原文件不删除。
fn backup_corrupt(path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "data.json".to_string());
    let backup = path.with_file_name(format!("{}.corrupt_{}", name, unix_ms()));
    fs::copy(path, &backup).map(|_| ()).map_err(|e| e.to_string())
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 原子写:先写唯一临时文件再 rename,避免断电或多线程并发互相覆盖。
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "data.json".to_string());
    let tmp = path.with_file_name(format!("{}.{}.tmp", name, uuid::Uuid::new_v4()));
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败: {e}"))?;
        let buf = serde_json::to_vec_pretty(value).map_err(|e| format!("序列化失败: {e}"))?;
        f.write_all(&buf).map_err(|e| format!("写入失败: {e}"))?;
        f.flush().map_err(|e| format!("刷新失败: {e}"))?;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("替换文件失败: {e}"));
    }
    Ok(())
}

/// 掩码:保留前4后4,中间用 … 代替。
pub fn mask(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return s.to_string();
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{}…{}", head, tail)
}

pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub fn today_prefix() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 追加一行到 data_dir/logs/app.log,用于关键路径排查。
pub fn app_log(data_dir: &Path, msg: &str) {
    let log_path = data_dir.join("logs").join("app.log");
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "[{}] {}", now_ts(), msg);
    }
}

pub fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_json_backs_up_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("trae_mate_a7_fs_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("remaining_credits.json");
        fs::write(&path, "{bad json").unwrap();

        let value: serde_json::Value = read_json(&path);
        assert!(value.is_null(), "解析失败返回默认值");

        let backups: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("remaining_credits.json.corrupt_"))
            .collect();
        assert_eq!(backups.len(), 1, "生成 corrupt 备份");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{bad json",
            "原文件未改动"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
