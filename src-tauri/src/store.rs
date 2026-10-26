// 本地数据存储:账号、日志、设置。持久化为 JSON(app_data_dir),原子写(临时文件+rename)。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::models::{Account, AppSettings, CheckinLog, PartialAppSettings};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoreData {
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub logs: Vec<CheckinLog>,
    #[serde(default)]
    pub settings: AppSettings,
}

/// 应用全局状态:数据 + 存储路径
pub struct AppState {
    pub data: Mutex<StoreData>,
    pub path: PathBuf,
    /// 应用数据根目录(app_data_dir),新增参考式 JSON 文件放其 data/ 子目录
    pub data_dir: PathBuf,
    /// JWT 刷新锁:防止多个并发请求同时 ExchangeToken(用 tokio Mutex,持锁可跨 await)
    pub jwt_refresh_lock: tokio::sync::Mutex<()>,
}

impl AppState {
    /// 参考式 JSON 数据文件路径:data_dir/data/{name},自动建目录。
    pub fn data_path(&self, name: &str) -> PathBuf {
        let dir = self.data_dir.join("data");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }
}

/// 账号的有效 user_id:desktop 用 desktop_user_id,jwt 用 user_id。
/// 用于 device_map / cooldown / credits 等按用户维度的数据文件索引。
pub fn effective_user_id(a: &Account) -> Option<&str> {
    match a.source {
        crate::models::AccountSource::Jwt => a.user_id.as_deref(),
        crate::models::AccountSource::Desktop => a.desktop_user_id.as_deref(),
    }
    .filter(|s| !s.is_empty())
}

impl StoreData {
    pub fn load(path: &Path) -> AppResult<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let store: StoreData = serde_json::from_str(&raw)?;
        Ok(store)
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn get_accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// 按 desktopUserId upsert:命中则以 incoming 合并进既有记录(保留身份/签到态/绑定信息),否则新增。
    pub fn upsert_desktop_account(&mut self, mut incoming: Account) -> Account {
        if let Some(i) = self
            .accounts
            .iter()
            .position(|a| a.desktop_user_id == incoming.desktop_user_id)
        {
            let old = self.accounts[i].clone();
            // 身份与签到态:导入不覆盖,保留既有值
            incoming.id = old.id;
            incoming.created_at = old.created_at;
            incoming.name = old.name;
            incoming.enabled = old.enabled;
            incoming.points = old.points;
            incoming.last_checkin_at = old.last_checkin_at;
            incoming.last_checkin_result = old.last_checkin_result;
            incoming.last_checkin_message = old.last_checkin_message;
            // 绑定信息:incoming 未提供(None)时保留旧值,避免导入清空实例目录/机器码
            if incoming.data_dir.is_none() {
                incoming.data_dir = old.data_dir;
            }
            if incoming.machine_id.is_none() {
                incoming.machine_id = old.machine_id;
            }
            // jwt 相关字段:incoming 未提供时保留旧值
            if incoming.user_id.is_none() {
                incoming.user_id = old.user_id;
            }
            if incoming.encrypted_jwt.is_none() {
                incoming.encrypted_jwt = old.encrypted_jwt;
            }
            if incoming.jwt_exp_timestamp.is_none() {
                incoming.jwt_exp_timestamp = old.jwt_exp_timestamp;
            }
            if incoming.jwt_status.is_none() {
                incoming.jwt_status = old.jwt_status;
            }
            if !incoming.has_refresh_token {
                incoming.has_refresh_token = old.has_refresh_token;
            }
            if !incoming.jwt_auto_refresh {
                incoming.jwt_auto_refresh = old.jwt_auto_refresh;
            }
            self.accounts[i] = incoming.clone();
            incoming
        } else {
            self.accounts.push(incoming.clone());
            incoming
        }
    }

    pub fn delete_account(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
    }

    /// 用 JSON 对象合并更新账号字段(前端传 partial,任意字段)。
    pub fn update_account(&mut self, id: &str, updates: serde_json::Value) -> Option<Account> {
        let i = self.accounts.iter().position(|a| a.id == id)?;
        let mut cur = serde_json::to_value(&self.accounts[i]).ok()?;
        if let (Some(obj), Some(upd)) = (cur.as_object_mut(), updates.as_object()) {
            for (k, v) in upd {
                obj.insert(k.clone(), v.clone());
            }
        }
        let merged: Account = serde_json::from_value(cur).ok()?;
        self.accounts[i] = merged.clone();
        Some(merged)
    }

    /// 返回最近 limit 条日志,最新在前(对应原 slice(-limit).reverse())
    pub fn get_logs(&self, limit: usize) -> Vec<CheckinLog> {
        let n = self.logs.len();
        let start = n.saturating_sub(limit);
        self.logs[start..].iter().rev().cloned().collect()
    }

    pub fn add_log(&mut self, log: CheckinLog) {
        self.logs.push(log);
        if self.logs.len() > 500 {
            let drop_n = self.logs.len() - 500;
            self.logs.drain(0..drop_n);
        }
    }

    pub fn clear_logs(&mut self) {
        self.logs.clear();
    }

    pub fn get_settings(&self) -> AppSettings {
        self.settings.clone()
    }

    pub fn save_settings(&mut self, partial: PartialAppSettings) -> AppSettings {
        self.settings.merge(partial);
        self.settings.clone()
    }
}

/// 生成简单唯一 ID(对应原 Date.now().toString(36) + random)
pub fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // 用进程内计数器补充随机性(Math.random 替代)
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{}", ms, n)
}

// 显式标注:AppState 内部用 Mutex,AppError 路径错误也归为 IO
#[allow(dead_code)]
fn _unused(_e: AppError) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AccountSource;

    fn sample_account(data_dir: Option<&str>, credential: &str, status: &str) -> Account {
        Account {
            id: "keep-id".into(),
            name: "用户53752712600".into(),
            cookie: String::new(),
            created_at: 1_761_367_507_000,
            last_checkin_at: Some(1_761_448_463_000),
            last_checkin_result: Some("success".into()),
            last_checkin_message: Some("签到成功".into()),
            points: Some(100),
            enabled: false,
            desktop_user_id: Some("2257768460321738".into()),
            encrypted_credential: Some(credential.into()),
            credential_status: Some(status.into()),
            data_dir: data_dir.map(|s| s.to_string()),
            machine_id: Some("machine-1".into()),
            source: AccountSource::Desktop,
            ..Default::default()
        }
    }

    #[test]
    fn upsert_preserves_identity_and_binding() {
        let mut store = StoreData::default();
        store
            .accounts
            .push(sample_account(Some(r"C:\X\TRAE SOLO CN_2257768460321738"), "old-cred", "expired"));
        let count_before = store.accounts.len();

        // 模拟导入:dataDir=None、机器码=None、凭据状态变为 valid
        let saved = store.upsert_desktop_account(sample_account(None, "new-cred", "valid"));

        assert_eq!(store.accounts.len(), count_before, "账号数量不变");
        assert_eq!(saved.id, "keep-id", "id 保留");
        assert_eq!(saved.name, "用户53752712600", "name 保留");
        assert_eq!(saved.created_at, 1_761_367_507_000, "createdAt 保留");
        assert!(!saved.enabled, "enabled 保留");
        assert_eq!(saved.points, Some(100), "points 保留");
        assert_eq!(saved.last_checkin_at, Some(1_761_448_463_000), "lastCheckinAt 保留");
        assert_eq!(saved.last_checkin_result.as_deref(), Some("success"), "lastCheckinResult 保留");
        assert_eq!(saved.last_checkin_message.as_deref(), Some("签到成功"), "lastCheckinMessage 保留");
        assert_eq!(
            saved.data_dir.as_deref(),
            Some(r"C:\X\TRAE SOLO CN_2257768460321738"),
            "dataDir 未提供时保留旧值"
        );
        assert_eq!(saved.machine_id.as_deref(), Some("machine-1"), "machineId 未提供时保留旧值");
        assert_eq!(saved.encrypted_credential.as_deref(), Some("new-cred"), "凭据被更新");
        assert_eq!(saved.credential_status.as_deref(), Some("valid"), "凭据状态被更新");
    }
}
