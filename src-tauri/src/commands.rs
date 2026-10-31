// Tauri 命令实现。

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::checkin;
use crate::credentials;
use crate::error::{AppError, AppResult};
use crate::models::{
    credential_status, Account, AccountSource, AppSettings, CheckinLog, CheckinResult, Credential,
    JwtCredential, LaunchResult, PartialAppSettings, PointsResult, PublicAccount,
};
use crate::scheduler;
use crate::store::{effective_user_id, generate_id, AppState};
use crate::trae_auth;
use crate::trae_instance;
use crate::trae_machine;

#[tauri::command]
pub fn get_accounts(state: State<'_, AppState>) -> Vec<PublicAccount> {
    build_account_views(&state)
}

/// 构建账号视图:聚合 device_map / 冷却 / 剩余积分 / JWT 状态 / 今日签到(对应参考 build_account_views)。
fn build_account_views(state: &AppState) -> Vec<PublicAccount> {
    use std::collections::HashSet;
    let now_ts = chrono::Utc::now().timestamp();
    let device_map = crate::device_map::load_device_map(state);
    let cooldowns = crate::cooldown::all_cooldowns(state, now_ts);
    let rc: crate::models::RemainingCreditsFile =
        crate::fs_utils::read_json(&state.data_path("remaining_credits.json"));
    let summary: crate::models::CheckinSummary =
        crate::fs_utils::read_json(&state.data_path("checkin_summary.json"));
    let today = crate::fs_utils::today_prefix();
    let summary_today = summary
        .time
        .as_deref()
        .map(|t| t.starts_with(&today))
        .unwrap_or(false);
    let checked_names: HashSet<String> = if summary_today {
        summary
            .results
            .iter()
            .filter(|r| {
                let ok = r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                ok || r.get("message").and_then(|v| v.as_str()).is_some()
            })
            .filter_map(|r| r.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect()
    } else {
        HashSet::new()
    };

    let accounts: Vec<Account> = {
        let data = state.data.lock().unwrap();
        data.get_accounts().to_vec()
    };

    accounts
        .into_iter()
        .map(|mut a| {
            let uid_owned = effective_user_id(&a).map(String::from);
            if let Some(uid) = uid_owned.as_deref() {
                // JWT 过期状态(仅 jwt 账号)
                if a.source == AccountSource::Jwt {
                    if let Some(sec) = a
                        .encrypted_jwt
                        .as_deref()
                        .and_then(|e| credentials::decrypt_jwt_secret(e).ok())
                    {
                        let info = crate::jwt::parse(&sec.jwt);
                        a.jwt_exp_timestamp = info.exp_timestamp;
                        a.jwt_status = Some(crate::jwt::status_of(info.exp_hours).to_string());
                        let has_rt = sec
                            .refresh_token
                            .as_deref()
                            .map(|r| !r.is_empty())
                            .unwrap_or(false);
                        a.has_refresh_token = has_rt;
                        a.jwt_auto_refresh = has_rt
                            && info.exp_hours.map(|h| h <= 24.0).unwrap_or(true);
                    }
                } else if let Some(status) = a.credential_status.clone() {
                    a.jwt_status = Some(status);
                }
                // 剩余积分 / 过期时间
                a.remaining_credits = rc.credits.get(uid).copied();
                a.credits_expire_at = rc.expire_times.get(uid).copied();
                // 设备标识(脱敏)
                a.device_id_masked =
                    device_map.get(uid).map(|d| crate::fs_utils::mask(&d.device_id));
                // 冷却状态
                if let Some((_, entry)) = cooldowns.iter().find(|(u, _)| u == uid) {
                    a.cooldown_type = Some(entry.error_type.clone());
                    a.cooldown_until = Some(entry.until);
                    a.cooldown_reason =
                        if entry.reason.is_empty() { None } else { Some(entry.reason.clone()) };
                }
                // 今日签到
                a.checked_today = Some(summary_today && checked_names.contains(&a.name));
                // 积分展示:优先 remaining_credits → credits_history → 既有值
                if let Some(c) = rc.credits.get(uid) {
                    a.points = Some(*c as i64);
                } else if let Some(c) = crate::credits::latest_credits_of(state, uid) {
                    a.points = Some(c);
                }
            }
            let dir = a.data_dir.clone();
            let mut view: PublicAccount = a.into();
            // 只读判断实例是否仍登着(不改任何 storage 内容)
            view.instance_logged_in = instance_logged_in(dir.as_deref());
            view
        })
        .collect()
}

/// 只读判断:实例目录的 storage.json 里 iCubeAuthInfo 是否存在且非空(即客户端仍登着)
fn instance_logged_in(data_dir: Option<&str>) -> Option<bool> {
    let dir = data_dir?;
    let path = std::path::Path::new(dir)
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    // 实际键名为 iCubeAuthInfo://icube.cloudide(个别版本写作 cube.cloudide),两者都认
    let ok = ["iCubeAuthInfo://icube.cloudide", "iCubeAuthInfo://cube.cloudide"]
        .iter()
        .any(|k| {
            value
                .get(*k)
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
        });
    Some(ok)
}

/// 导入当前 TRAE 桌面账号:读取桌面凭据 -> DPAPI 加密 -> upsert 存储
#[tauri::command]
pub fn import_desktop_account(state: State<'_, AppState>) -> AppResult<PublicAccount> {
    let cred = trae_auth::get_trae_desktop_credentials()?;
    let encrypted = credentials::encrypt_credential(&cred)?;
    let now = now_ms();
    let status = credential_status(cred.expires_at, now);
    let account = Account {
        id: generate_id(),
        name: cred.account_name.clone(),
        cookie: String::new(),
        created_at: now,
        last_checkin_at: None,
        last_checkin_result: None,
        last_checkin_message: None,
        points: None,
        enabled: true,
        desktop_user_id: Some(cred.user_id.clone()),
        encrypted_credential: Some(encrypted),
        credential_status: Some(status.to_string()),
        data_dir: trae_machine::instance_data_dir_for_current_user(&cred.user_id)
            .map(|p| p.to_string_lossy().to_string()),
        machine_id: None,
        source: AccountSource::Desktop,
        ..Default::default()
    };
    let mut data = state.data.lock().unwrap();
    let saved = data.upsert_desktop_account(account);
    let _ = data.save(&state.path);
    Ok(saved.into())
}

#[tauri::command]
pub fn update_account(
    id: String,
    updates: serde_json::Value,
    state: State<'_, AppState>,
) -> AppResult<PublicAccount> {
    let mut data = state.data.lock().unwrap();
    // jwt 账号:更新 jwt/refreshToken 时重新加密写回 encrypted_jwt,并同步 user_id
    let current = data
        .get_accounts()
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let mut updates = updates;
    if current.source == AccountSource::Jwt {
        let mut sec = current
            .encrypted_jwt
            .as_deref()
            .and_then(|e| credentials::decrypt_jwt_secret(e).ok())
            .unwrap_or_else(|| JwtCredential {
                jwt: String::new(),
                refresh_token: None,
            });
        if let Some(j) = updates.get("jwt").and_then(|v| v.as_str()) {
            let j = j.trim().to_string();
            if !j.is_empty() {
                let info = crate::jwt::parse(&j);
                if let Some(uid) = info.user_id {
                    if let Some(obj) = updates.as_object_mut() {
                        obj.insert("user_id".into(), serde_json::json!(uid));
                    }
                }
                sec.jwt = crate::jwt::normalize_full(&j);
            }
        }
        if let Some(rt) = updates.get("refreshToken").and_then(|v| v.as_str()) {
            let t = rt.trim().to_string();
            sec.refresh_token = if t.is_empty() { None } else { Some(t) };
        }
        let encrypted = credentials::encrypt_jwt_secret(&sec)?;
        if let Some(obj) = updates.as_object_mut() {
            obj.insert("encryptedJwt".into(), serde_json::json!(encrypted));
            obj.remove("jwt");
            obj.remove("refreshToken");
        }
    }
    let acc = data
        .update_account(&id, updates)
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    data.save(&state.path)?;
    Ok(acc.into())
}

#[tauri::command]
pub fn delete_account(id: String, state: State<'_, AppState>, app: AppHandle) -> AppResult<bool> {
    // 关闭该账号的工具实例(若在运行),数据目录保留(用户可日后通过"扫描已有多开目录"重新导入)
    let uid_to_cleanup = {
        let data = state.data.lock().unwrap();
        if let Some(acc) = data.get_accounts().iter().find(|a| a.id == id) {
            if let Some(dir) = acc.data_dir.as_deref().filter(|s| !s.is_empty()) {
                if trae_machine::is_instance_running(dir).0 {
                    let _ = trae_machine::kill_instance(dir);
                }
            }
            effective_user_id(acc).map(|s| s.to_string())
        } else {
            None
        }
    };
    let mut data = state.data.lock().unwrap();
    data.delete_account(&id);
    data.save(&state.path)?;
    drop(data);
    // 联动清理参考式数据文件中的该账号条目(避免残留冷却/设备记录误伤重加账号)
    if let Some(uid) = uid_to_cleanup {
        crate::device_map::reset_device_for(&state, &uid);
        crate::cooldown::remove_cooldown(&state, &uid);
        crate::credits::remove_account_data(&state, &uid);
    }
    // 刷新托盘菜单:移除已删除账号的快捷聚焦项
    let _ = crate::rebuild_tray_menu(&app);
    Ok(true)
}

#[tauri::command]
pub async fn checkin_account(
    id: String,
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
    app: AppHandle,
) -> AppResult<CheckinResult> {
    let account = {
        let data = state.data.lock().unwrap();
        data.get_accounts().iter().find(|a| a.id == id).cloned()
    };
    let account = account.ok_or_else(|| AppError::NotFound(id.clone()))?;
    Ok(checkin::perform_checkin(Some(&app), &account, client.inner(), state.inner()).await)
}

#[tauri::command]
pub async fn checkin_all(
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
    app: AppHandle,
) -> AppResult<Vec<(PublicAccount, CheckinResult)>> {
    // 手动一键签到:账号间保持 2s 快速执行(自动定时签到用可配置的分钟级间隔,见 scheduler)
    // 手动不跳过当天已签账号,保证用户主动点击时能看到每个账号的真实状态
    checkin::perform_all_checkin(&app, client.inner(), state.inner(), 2, false)
        .await
        .map_err(AppError::Launch)
}

#[tauri::command]
pub async fn get_account_points(
    id: String,
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
) -> AppResult<PointsResult> {
    let account = {
        let data = state.data.lock().unwrap();
        data.get_accounts().iter().find(|a| a.id == id).cloned()
    };
    let account = account.ok_or_else(|| AppError::NotFound(id.clone()))?;
    let result = checkin::get_total_points(&account, client.inner(), state.inner()).await;
    if let (true, Some(tp)) = (result.success, result.total_points) {
        let mut data = state.data.lock().unwrap();
        data.update_account(&id, serde_json::json!({ "points": tp }));
        let _ = data.save(&state.path);
    }
    Ok(result)
}

/// 按 user_id 查找账号
fn find_by_user_id(state: &AppState, user_id: &str) -> Option<Account> {
    let data = state.data.lock().unwrap();
    data.get_accounts()
        .iter()
        .find(|a| {
            effective_user_id(a).map(|u| u == user_id).unwrap_or(false)
                || a.desktop_user_id.as_deref() == Some(user_id)
        })
        .cloned()
}

/// 手动录入 JWT 账号(参考模式):解析 user_id -> 查重 -> DPAPI 加密 -> 保存
#[tauri::command]
pub fn account_add_jwt(
    state: State<'_, AppState>,
    name: String,
    jwt: String,
    refresh_token: Option<String>,
    enabled: Option<bool>,
) -> AppResult<PublicAccount> {
    let info = crate::jwt::parse(&jwt);
    let uid = info
        .user_id
        .ok_or_else(|| AppError::Credential("无法从 JWT 解析 user_id,请检查格式".into()))?;
    let mut data = state.data.lock().unwrap();
    if data.get_accounts().iter().any(|a| {
        a.desktop_user_id.as_deref() == Some(uid.as_str())
            || a.user_id.as_deref() == Some(uid.as_str())
    }) {
        return Err(AppError::Credential("该账号已存在".into()));
    }
    let sec = JwtCredential {
        jwt: crate::jwt::normalize_full(&jwt),
        refresh_token,
    };
    let encrypted = credentials::encrypt_jwt_secret(&sec)?;
    let account = Account {
        id: generate_id(),
        name,
        cookie: String::new(),
        created_at: now_ms(),
        enabled: enabled.unwrap_or(true),
        source: AccountSource::Jwt,
        user_id: Some(uid.clone()),
        encrypted_jwt: Some(encrypted),
        ..Default::default()
    };
    let saved = data.upsert_desktop_account(account);
    let _ = data.save(&state.path);
    Ok(saved.into())
}

/// JWT 解析预览(弹窗实时显示 user_id / 剩余小时)
#[tauri::command]
pub fn jwt_parse_preview(jwt: String) -> Result<crate::jwt::JwtInfo, String> {
    let info = crate::jwt::parse(&jwt);
    if info.user_id.is_none() {
        return Err("无法从 JWT 解析 user_id,请检查格式".into());
    }
    Ok(info)
}

/// 手动刷新 jwt 账号的 JWT(refresh_token -> ExchangeToken),返回更新后账号
#[tauri::command]
pub async fn refresh_jwt_account(
    user_id: String,
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
) -> AppResult<PublicAccount> {
    let account =
        find_by_user_id(&state, &user_id).ok_or_else(|| AppError::NotFound(user_id.clone()))?;
    if account.source != AccountSource::Jwt {
        return Err(AppError::Credential("仅 JWT 账号可手动刷新".into()));
    }
    let _guard = state.jwt_refresh_lock.lock().await;
    // 持锁后重读(防并发)
    let current = find_by_user_id(&state, &user_id)
        .ok_or_else(|| AppError::NotFound(user_id.clone()))?;
    let enc = current
        .encrypted_jwt
        .as_deref()
        .ok_or_else(|| AppError::Credential("该账号无 JWT 凭据".into()))?;
    let sec = credentials::decrypt_jwt_secret(enc)?;
    let rt = sec
        .refresh_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Credential("该账号无 refresh_token,无法自动刷新".into()))?
        .to_string();
    let (new_jwt, new_rt) = crate::jwt::exchange_token(&rt, client.inner())
        .await
        .map_err(AppError::Credential)?;
    // 校验 user_id 一致
    let new_uid = crate::jwt::parse(&new_jwt).user_id;
    if let Some(nu) = new_uid.as_deref() {
        if nu != user_id {
            return Err(AppError::Credential(format!(
                "刷新后 user_id 不匹配: 期望={user_id}, 实际={nu}"
            )));
        }
    }
    let mut sec2 = sec;
    sec2.jwt = new_jwt;
    if let Some(rt2) = new_rt {
        sec2.refresh_token = Some(rt2);
    }
    let encrypted = credentials::encrypt_jwt_secret(&sec2)?;
    let mut data = state.data.lock().unwrap();
    let acc = data
        .update_account(
            &current.id,
            serde_json::json!({ "encryptedJwt": encrypted, "user_id": new_uid }),
        )
        .ok_or_else(|| AppError::NotFound(current.id.clone()))?;
    let _ = data.save(&state.path);
    Ok(acc.into())
}

/// 清除单个账号冷却状态
#[tauri::command]
pub fn cooldown_clear(state: State<'_, AppState>, user_id: String) -> AppResult<()> {
    crate::cooldown::clear_cooldown(&state, &user_id);
    Ok(())
}

/// 清除所有账号冷却状态
#[tauri::command]
pub fn cooldown_clear_all(state: State<'_, AppState>) -> AppResult<usize> {
    Ok(crate::cooldown::clear_all_cooldowns(&state))
}

/// 重置某账号的伪设备身份(下次签到重新派生)
#[tauri::command]
pub fn device_reset(state: State<'_, AppState>, user_id: String) -> AppResult<()> {
    crate::device_map::reset_device_for(&state, &user_id);
    Ok(())
}

/// 实时查询某账号剩余积分(写 remaining_credits.json 缓存)
#[tauri::command]
pub async fn fetch_remaining_credits(
    user_id: String,
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
) -> AppResult<f64> {
    let account =
        find_by_user_id(&state, &user_id).ok_or_else(|| AppError::NotFound(user_id.clone()))?;
    let jwt = crate::credits::resolve_account_jwt(&account)
        .ok_or_else(|| AppError::Credential("账号无可用凭据".into()))?;
    let (credits, expire_at, _) =
        crate::credits::calc_remaining_credits(&jwt, client.inner())
            .await
            .map_err(AppError::Credential)?;
    let mut rc: crate::models::RemainingCreditsFile =
        crate::fs_utils::read_json(&state.data_path("remaining_credits.json"));
    rc.credits.insert(user_id.clone(), credits);
    if let Some(exp) = expire_at {
        rc.expire_times.insert(user_id.clone(), exp);
    }
    rc.updated_at = Some(crate::fs_utils::now_iso());
    let _ = crate::fs_utils::write_json(&state.data_path("remaining_credits.json"), &rc);
    Ok(credits)
}

/// 刷新所有账号剩余积分(含自动解冻)
#[tauri::command]
pub async fn refresh_all_remaining_credits(
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
) -> AppResult<usize> {
    Ok(crate::credits::refresh_remaining_credits(&state, client.inner())
        .await
        .map_err(AppError::Credential)?)
}

/// 每日积分快照列表(积分看板三线趋势数据源)
/// 同时返回 remaining_credits.json 的 updated_at,供前端显示「数据截至」
#[tauri::command]
pub fn credits_daily_list(state: State<'_, AppState>) -> serde_json::Value {
    let file: crate::models::CreditsDailyFile =
        crate::fs_utils::read_json(&state.data_path("credits_daily.json"));
    let rc: crate::models::RemainingCreditsFile =
        crate::fs_utils::read_json(&state.data_path("remaining_credits.json"));
    serde_json::json!({
        "snapshots": file.snapshots,
        "dataUpdatedAt": rc.updated_at,
    })
}

/// 导出积分数据为 CSV(日汇总 + 每账号明细),UTF-8 无 BOM + CRLF,Excel 可直接打开。
/// 返回两个文件的绝对路径(顺序:日汇总、明细)。
#[tauri::command]
pub fn credits_export_csv(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    let file: crate::models::CreditsDailyFile =
        crate::fs_utils::read_json(&state.data_path("credits_daily.json"));
    let export_dir = state.data_path("exports");
    std::fs::create_dir_all(&export_dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M").to_string();

    let mut snaps = file.snapshots.clone();
    snaps.sort_by(|a, b| a.date.cmp(&b.date));

    let mut csv_daily = String::from("date,total,earned,consumed,legacy\r\n");
    for s in &snaps {
        // 旧数据(无 per_account)一律视为旧算法算出
        let legacy = s.consumed_legacy || s.per_account.is_none();
        csv_daily.push_str(&format!(
            "{},{:.2},{:.2},{:.2},{}\r\n",
            s.date,
            s.total,
            s.earned,
            s.consumed,
            if legacy { "true" } else { "false" }
        ));
    }
    let daily_path = export_dir.join(format!("credits_daily_{stamp}.csv"));
    std::fs::write(&daily_path, csv_daily.as_bytes())?;

    // 账号名映射:remaining_credits 的 key 与 effective_user_id 一致
    let names: std::collections::HashMap<String, String> = {
        let data = state.data.lock().unwrap();
        data.get_accounts()
            .iter()
            .filter_map(|a| effective_user_id(a).map(|uid| (uid.to_string(), a.name.clone())))
            .collect()
    };
    let mut csv_detail = String::from("date,uid,name,credits\r\n");
    for s in &snaps {
        let Some(map) = &s.per_account else {
            continue;
        };
        for (uid, credits) in map {
            let name = names.get(uid).cloned().unwrap_or_default();
            csv_detail.push_str(&format!(
                "{},{},{},{:.2}\r\n",
                s.date,
                uid,
                csv_field(&name),
                credits
            ));
        }
    }
    let detail_path = export_dir.join(format!("credits_detail_{stamp}.csv"));
    std::fs::write(&detail_path, csv_detail.as_bytes())?;

    Ok(vec![
        daily_path.to_string_lossy().to_string(),
        detail_path.to_string_lossy().to_string(),
    ])
}

/// CSV 字段转义(含逗号/引号/换行时加双引号,内部引号加倍)
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 用系统默认浏览器打开外部链接(关于页"项目地址"用)
#[tauri::command]
pub fn open_url(url: String) -> AppResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| AppError::Launch(format!("打开链接失败: {e}")))?;
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
    Ok(())
}

#[tauri::command]
pub fn get_logs(limit: Option<usize>, state: State<'_, AppState>) -> Vec<CheckinLog> {
    let data = state.data.lock().unwrap();
    data.get_logs(limit.unwrap_or(100))
}

#[tauri::command]
pub fn clear_logs(state: State<'_, AppState>) -> AppResult<bool> {
    let mut data = state.data.lock().unwrap();
    data.clear_logs();
    data.save(&state.path)?;
    Ok(true)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>, app: AppHandle) -> AppSettings {
    let mut s = {
        let data = state.data.lock().unwrap();
        data.get_settings()
    };
    // 用插件真实状态覆盖(反映系统设置/任务管理器等外部修改)
    use tauri_plugin_autostart::ManagerExt;
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        s.launch_at_login = enabled;
    }
    s
}

#[tauri::command]
pub fn save_settings(
    settings: PartialAppSettings,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<AppSettings> {
    let has_launch = settings.launch_at_login.is_some();
    // 先校验签到时间:非法配置直接拒绝,不写盘也不重启调度
    if let Some(t) = settings.checkin_time.as_deref() {
        if !scheduler::validate_hhmm(t) {
            return Err(AppError::Launch(
                "签到时间格式无效,应为 HH:MM(00:00-23:59)".into(),
            ));
        }
    }
    let mut data = state.data.lock().unwrap();
    let s = data.save_settings(settings);
    data.save(&state.path)?;
    drop(data);
    // 同步开机自启插件状态(仅在本次提交了该字段时)
    if has_launch {
        use tauri_plugin_autostart::ManagerExt;
        let mgr = app.autolaunch();
        let currently = mgr.is_enabled().unwrap_or(false);
        if s.launch_at_login && !currently {
            let _ = mgr.enable();
        } else if !s.launch_at_login && currently {
            let _ = mgr.disable();
        }
    }
    // 设置变更后重启定时任务
    scheduler::start_scheduler(app);
    Ok(s)
}

#[tauri::command]
pub fn start_scheduler(app: AppHandle) -> bool {
    scheduler::start_scheduler(app);
    true
}

#[tauri::command]
pub fn stop_scheduler(app: AppHandle) -> bool {
    scheduler::stop_scheduler(&app);
    true
}

#[tauri::command]
pub fn get_next_run_time(app: AppHandle) -> Option<String> {
    scheduler::get_next_run_time(&app)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 账号是否已有可用的绑定实例目录(目录内存在 User\globalStorage\storage.json)。
/// 可用时必须用该目录启动,否则会落到主目录、看不到该账号的聊天记录。
fn bound_dir_usable(account: &Account) -> bool {
    account
        .data_dir
        .as_deref()
        .filter(|d| !d.is_empty())
        .map(|d| {
            std::path::Path::new(d)
                .join("User")
                .join("globalStorage")
                .join("storage.json")
                .is_file()
        })
        .unwrap_or(false)
}

/// 启动账号独立实例(前端命令与托盘点击共用核心逻辑):
/// 解密凭据 -> 解析 exe -> 启动 -> 回写 data_dir/machine_id -> 刷新托盘菜单
pub fn launch_account_by_id(app: &AppHandle, account_id: &str) -> AppResult<LaunchResult> {
    let state = app.state::<AppState>();
    // 1. 取账号
    let account = {
        let data = state.data.lock().unwrap();
        data.get_accounts()
            .iter()
            .find(|a| a.id == account_id)
            .cloned()
    }
    .ok_or_else(|| AppError::NotFound(account_id.into()))?;

    // 已有可用绑定目录时一律用绑定目录启动(含主账号),保证聊天记录/登录态落在同一目录
    let use_bound_dir = bound_dir_usable(&account);

    // 防护:主目录正登录该账号时一律拒绝(无论有无绑定目录);双设备身份会让 refresh 被判定 token device not match
    let main = trae_machine::probe_main_instance();
    if matches!(
        trae_machine::account_state(&account, &main),
        trae_machine::InstanceSource::Main
    ) {
        return Err(AppError::Launch(
            "主目录 Trae CN 正登录该账号,请先在主目录退出登录再启动,否则同账号双设备身份会导致令牌失效".into(),
        ));
    }

    // 2. 解析 exe 路径(已保存或自动扫描)
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Launch(format!("获取配置目录失败: {e}")))?;
    let exe_path = trae_machine::resolve_trae_path(&config_dir)?;

    // 3. 无可用绑定目录的主实例账号用主目录启动,复用主实例登录态;其余一律独立目录多开。
    //    主目录 userId 取自主目录 storage.json(主实例关闭后仍残留最后登录账号),实现"固定绑定主目录"。
    let is_main_account = !use_bound_dir
        && main.1.as_deref()
            == account
                .desktop_user_id
                .as_deref()
                .filter(|s| !s.is_empty());
    if is_main_account {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| AppError::Launch("无法获取 APPDATA 环境变量".into()))?;
        let main_dir = trae_machine::main_data_dir()?;
        let shared_ext = PathBuf::from(&appdata)
            .join(trae_instance::SHARED_EXTENSIONS_DIR)
            .to_string_lossy()
            .to_string();
        trae_machine::open_product_with_data_dir(
            &exe_path,
            &main_dir.to_string_lossy(),
            Some(&shared_ext),
        )?;
        let _ = crate::rebuild_tray_menu(app);
        return Ok(LaunchResult {
            data_dir: main_dir.to_string_lossy().to_string(),
            machine_id: String::new(),
            launched: true,
        });
    }

    // 4. 独立目录多开:解密凭据 -> 回读实例目录最新凭据(TRAE 可能自己刷新过,避免旧快照覆盖) -> launch_multi(写凭据到 TRAE SOLO CN_{userId})
    let encrypted = account
        .encrypted_credential
        .as_ref()
        .ok_or_else(|| AppError::Launch("该账号尚未导入 TRAE 桌面凭证,无法启动".into()))?;
    let decrypted = credentials::decrypt_credential(encrypted)?;
    let (cred, adopted) = trae_instance::sync_credential_from_instance(&account, &decrypted);
    // 回读到更新凭据时同步应用存储,下次签到直接用新值
    if adopted {
        let encrypted_new = credentials::encrypt_credential(&cred)?;
        // 同步凭证状态:仅回写 token 不更新 credentialStatus 会让卡片在重新登录后
        // 仍显示"凭证已失效"(旧状态残留到下次签到/手动刷新)
        let status = credential_status(cred.expires_at, now_ms());
        let mut data = state.data.lock().unwrap();
        let _ = data.update_account(
            account_id,
            serde_json::json!({ "encryptedCredential": encrypted_new, "credentialStatus": status }),
        );
        let _ = data.save(&state.path);
    }
    // 其他账号已占用的机器码:避免本实例复用同一设备身份(多账号共用设备会触发 4017 设备绑定上限)
    let taken_machine_ids: Vec<String> = {
        let data = state.data.lock().unwrap();
        data.get_accounts()
            .iter()
            .filter(|a| a.id != account_id)
            .filter_map(|a| a.machine_id.clone())
            .filter(|m| !m.is_empty())
            .collect()
    };
    let result = trae_instance::launch_multi(&account, &cred, &exe_path, &taken_machine_ids)?;

    // 5. 回写 data_dir/machine_id(首次启动时绑定,后续复用同一目录与机器码)
    {
        let mut data = state.data.lock().unwrap();
        data.update_account(
            account_id,
            serde_json::json!({
                "dataDir": result.data_dir,
                "machineId": result.machine_id,
            }),
        );
        let _ = data.save(&state.path);
    }

    // 6. 刷新托盘菜单:新实例已运行,状态前缀更新
    let _ = crate::rebuild_tray_menu(app);

    Ok(result)
}

/// 启动账号独立实例(前端命令入口)
#[tauri::command]
pub fn launch_account_multi(id: String, app: AppHandle) -> AppResult<LaunchResult> {
    launch_account_by_id(&app, &id)
}

/// 获取已保存的 TRAE exe 路径(未设置返回 None)
#[tauri::command]
pub fn get_trae_exe_path(app: AppHandle) -> AppResult<Option<String>> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Launch(format!("获取配置目录失败: {e}")))?;
    trae_machine::get_saved_trae_path(&config_dir)
}

/// 手动设置 TRAE exe 路径
#[tauri::command]
pub fn set_trae_exe_path(path: String, app: AppHandle) -> AppResult<()> {
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Launch(format!("获取配置目录失败: {e}")))?;
    trae_machine::save_trae_path(&config_dir, &path)
}

/// 自动扫描 TRAE exe 路径并保存
#[tauri::command]
pub fn scan_trae_exe_path(app: AppHandle) -> AppResult<String> {
    let scanned = trae_machine::scan_trae_exe_path()?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Launch(format!("获取配置目录失败: {e}")))?;
    let _ = trae_machine::save_trae_path(&config_dir, &scanned);
    Ok(scanned)
}

/// 账号实例运行状态(返回前端)
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceState {
    pub running: bool,
    pub source: trae_machine::InstanceSource,
    pub is_main_account: bool,
}

/// 查询账号实例运行状态:未运行 / 主实例(用户手动启动的 TRAE)/ 工具实例(本应用启动的独立 data-dir)
#[tauri::command]
pub fn get_account_instance_state(
    id: String,
    state: State<'_, AppState>,
) -> AppResult<InstanceState> {
    let account = {
        let data = state.data.lock().unwrap();
        data.get_accounts()
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }
    .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let main = trae_machine::probe_main_instance();
    let source = trae_machine::account_state(&account, &main);
    let is_main_account = main.1.as_deref()
        == account
            .desktop_user_id
            .as_deref()
            .filter(|s| !s.is_empty());
    Ok(InstanceState {
        running: !matches!(source, trae_machine::InstanceSource::None),
        source,
        is_main_account,
    })
}

/// 聚焦账号实例窗口:工具实例运行聚焦其 data-dir 窗口,主实例运行聚焦主目录窗口,未运行则报错。
#[tauri::command]
pub fn focus_account_instance(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let account = {
        let data = state.data.lock().unwrap();
        data.get_accounts()
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }
    .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let main = trae_machine::probe_main_instance();
    match trae_machine::account_state(&account, &main) {
        trae_machine::InstanceSource::Tool => {
            let d = account
                .data_dir
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::Launch("该账号无工具实例数据目录".into()))?;
            trae_machine::focus_instance_window(d)
        }
        trae_machine::InstanceSource::Main => {
            let main_dir = trae_machine::main_data_dir()?;
            trae_machine::focus_instance_window(&main_dir.to_string_lossy())
        }
        trae_machine::InstanceSource::None => {
            Err(AppError::Launch("该账号尚未启动,无实例可聚焦".into()))
        }
    }
}

/// 把临时登录目录迁移为标准实例目录 `TRAE SOLO CN_{userId}`。
/// 返回 (实际目录, 是否已写入新凭据)。
/// 目标目录已存在时绝不删除既有内容:仅备份并覆盖 `storage.json`;
/// 目标实例正在运行时完全不写文件,保留临时目录并返回 false。
pub(crate) fn relocate_login_dir(
    appdata: &Path,
    temp_dir: &Path,
    user_id: &str,
    target_running: bool,
) -> AppResult<(PathBuf, bool)> {
    let target = appdata.join(format!("{}_{}", trae_machine::DATA_DIR_NAME, user_id));
    if !target.exists() {
        std::fs::rename(temp_dir, &target)?;
        return Ok((target, true));
    }
    if target_running {
        return Ok((target, false));
    }

    let temp_storage = temp_dir
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    let target_storage = target.join("User").join("globalStorage").join("storage.json");
    if temp_storage.exists() {
        if target_storage.exists() {
            let backup = target_storage.with_file_name(format!("storage.json.bak_{}", now_ms()));
            std::fs::copy(&target_storage, &backup)?;
        }
        if let Some(parent) = target_storage.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&temp_storage, &target_storage)?;
    }
    let _ = std::fs::remove_dir_all(temp_dir);
    Ok((target, true))
}

/// 打开新的空白 TRAE 实例供用户登录,后台轮询登录完成后自动导入账号:
/// 检测登录 -> 读凭据 -> 杀实例 -> 改名临时目录为标准 TRAE SOLO CN_{userId} -> upsert 账号绑定 -> emit 事件。
#[tauri::command]
pub fn open_new_login_instance(app: AppHandle) -> AppResult<()> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| AppError::Launch("无法获取 APPDATA 环境变量".into()))?;
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Launch(format!("获取配置目录失败: {e}")))?;
    let exe_path = trae_machine::resolve_trae_path(&config_dir)?;

    // 临时 data-dir(带 uuid 后缀,避免与标准目录或多次操作冲突)
    let temp_dir = PathBuf::from(&appdata)
        .join(format!(
            "{} login {}",
            trae_machine::DATA_DIR_NAME,
            uuid::Uuid::new_v4()
        ))
        .to_string_lossy()
        .to_string();
    let shared_ext = PathBuf::from(&appdata)
        .join(trae_instance::SHARED_EXTENSIONS_DIR)
        .to_string_lossy()
        .to_string();

    // 启动空白实例(不写凭据,用户自行登录)
    trae_machine::open_product_with_data_dir(&exe_path, &temp_dir, Some(&shared_ext))?;

    // 后台轮询登录 + 导入,完成后 emit 事件
    let app_handle = app.clone();
    let appdata_owned = appdata;
    let temp_dir_owned = temp_dir;
    std::thread::spawn(move || {
        let result = wait_login_and_import(&app_handle, &appdata_owned, &temp_dir_owned);
        let _ = app_handle.emit("login-imported", result);
    });
    Ok(())
}

/// 轮询临时实例登录态,登录后导入账号并改名目录。返回事件 payload。
fn wait_login_and_import(app: &AppHandle, appdata: &str, temp_dir: &str) -> serde_json::Value {
    let storage_path = PathBuf::from(temp_dir)
        .join("User")
        .join("globalStorage")
        .join("storage.json");

    // 轮询 iCubeAuthInfo 出现(2s 一次,最多 10 分钟)
    let mut logged_in = false;
    for _ in 0..300 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if storage_path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&storage_path) {
                if let Ok(storage) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if storage
                        .get("iCubeAuthInfo://icube.cloudide")
                        .and_then(|v| v.as_str())
                        .is_some()
                    {
                        logged_in = true;
                        break;
                    }
                }
            }
        }
    }
    if !logged_in {
        return serde_json::json!({
            "success": false,
            "error": "登录超时(10 分钟未检测到登录),临时目录已保留供手动处理"
        });
    }

    // 读凭据
    let cred = match trae_auth::read_credentials_from_data_dir(&PathBuf::from(temp_dir)) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "success": false,
                "error": format!("读取登录凭据失败: {e}")
            })
        }
    };
    let user_id = cred.user_id.clone();
    if user_id.is_empty() {
        return serde_json::json!({
            "success": false,
            "error": "登录凭据缺少 userId"
        });
    }

    // 杀实例并等退出(目录占用解除后才能改名)
    let _ = trae_machine::kill_instance(temp_dir);
    for _ in 0..20 {
        if !trae_machine::is_instance_running(temp_dir).0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // 迁移临时登录目录到标准目录:目标已存在时只备份并覆盖 storage.json,绝不删除既有实例目录
    let appdata_path = PathBuf::from(appdata);
    let temp_path = PathBuf::from(temp_dir);
    let target_path =
        appdata_path.join(format!("{}_{}", trae_machine::DATA_DIR_NAME, user_id));
    let target_running = trae_machine::is_instance_running(&target_path.to_string_lossy()).0;
    let (data_dir_path, credential_copied) =
        match relocate_login_dir(&appdata_path, &temp_path, &user_id, target_running) {
            Ok(v) => v,
            Err(e) => {
                return serde_json::json!({
                    "success": false,
                    "error": format!("迁移登录目录失败: {e}")
                })
            }
        };
    let data_dir = data_dir_path.to_string_lossy().to_string();
    let temp_kept = temp_path.exists();

    // upsert 账号(绑定标准目录 + 机器码 + 加密凭据)
    let encrypted = match credentials::encrypt_credential(&cred) {
        Ok(e) => e,
        Err(e) => {
            return serde_json::json!({
                "success": false,
                "error": format!("加密凭据失败: {e}")
            })
        }
    };
    let now = now_ms();
    let status = credential_status(cred.expires_at, now);
    let account = Account {
        id: generate_id(),
        name: cred.account_name.clone(),
        cookie: String::new(),
        created_at: now,
        last_checkin_at: None,
        last_checkin_result: None,
        last_checkin_message: None,
        points: None,
        enabled: true,
        desktop_user_id: Some(user_id.clone()),
        encrypted_credential: Some(encrypted),
        credential_status: Some(status.to_string()),
        data_dir: Some(data_dir.clone()),
        machine_id: Some(cred.machine_id.clone()),
        source: AccountSource::Desktop,
        ..Default::default()
    };
    {
        let state = app.state::<AppState>();
        let mut data = state.data.lock().unwrap();
        data.upsert_desktop_account(account);
        let _ = data.save(&state.path);
    }
    let _ = crate::rebuild_tray_menu(app);
    serde_json::json!({
        "success": true,
        "name": cred.account_name,
        "userId": user_id,
        "credentialCopied": credential_copied,
        "tempKept": temp_kept
    })
}

/// 手动刷新账号凭证(卡片"刷新凭证"按钮):实例目录回读 + ExchangeToken 刷新 + 回写
#[tauri::command]
pub async fn refresh_account_credential(
    id: String,
    state: State<'_, AppState>,
    client: State<'_, reqwest::Client>,
) -> AppResult<PublicAccount> {
    let account = {
        let data = state.data.lock().unwrap();
        data.get_accounts().iter().find(|a| a.id == id).cloned()
    }
    .ok_or_else(|| AppError::NotFound(id.clone()))?;
    checkin::refresh_account_credential(&account, client.inner(), state.inner()).await?;
    let acc = {
        let data = state.data.lock().unwrap();
        data.get_accounts()
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(id.clone()))?
    };
    Ok(acc.into())
}

/// 扫描 %APPDATA% 下已存在的多开/登录临时目录,标注是否已绑定应用内账号
#[tauri::command]
pub fn scan_instance_dirs(state: State<'_, AppState>) -> Vec<trae_instance::InstanceDirInfo> {
    let mut dirs = trae_instance::scan_instance_dirs();
    let accounts = {
        let data = state.data.lock().unwrap();
        data.get_accounts().to_vec()
    };
    for d in &mut dirs {
        d.bound = accounts.iter().any(|a| {
            a.data_dir.as_deref() == Some(d.data_dir.as_str())
                || (!d.user_id.is_empty()
                    && a.desktop_user_id.as_deref() == Some(d.user_id.as_str()))
        });
    }
    dirs
}

/// 从已有多开目录导入账号(扫描列表的"导入"入口):
/// 完整凭据优先,缺失签名密钥时退宽松读取(token 可用但过期后无法自动刷新)。
/// 同 userId 账号已存在则仅更新凭据并绑定目录,保留名称/积分/签到历史。
#[tauri::command]
pub fn import_account_from_dir(
    data_dir: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<PublicAccount> {
    let path = PathBuf::from(&data_dir);
    if !path
        .join("User")
        .join("globalStorage")
        .join("storage.json")
        .exists()
    {
        return Err(AppError::NotFound(format!("目录无 storage.json: {data_dir}")));
    }
    let cred = trae_auth::read_credentials_from_data_dir(&path)
        .or_else(|_| trae_auth::read_auth_from_data_dir_loose(&path, &Credential::empty()))?;
    if cred.user_id.is_empty() {
        return Err(AppError::Credential("目录登录信息缺少 userId".into()));
    }

    let encrypted = credentials::encrypt_credential(&cred)?;
    let now = now_ms();
    let status = credential_status(cred.expires_at, now);
    let machine_id = if cred.machine_id.is_empty() {
        None
    } else {
        Some(cred.machine_id.clone())
    };

    let mut data = state.data.lock().unwrap();
    let existing = data
        .get_accounts()
        .iter()
        .find(|a| a.desktop_user_id.as_deref() == Some(cred.user_id.as_str()))
        .cloned();
    let saved = match existing {
        Some(acc) => data
            .update_account(
                &acc.id,
                serde_json::json!({
                    "encryptedCredential": encrypted,
                    "credentialStatus": status,
                    "dataDir": data_dir,
                    "machineId": machine_id,
                    "expiresAt": cred.expires_at,
                    "refreshExpiresAt": cred.refresh_expires_at,
                }),
            )
            .ok_or_else(|| AppError::NotFound(acc.id.clone()))?,
        None => {
            let account = Account {
                id: generate_id(),
                name: cred.account_name.clone(),
                cookie: String::new(),
                created_at: now,
                last_checkin_at: None,
                last_checkin_result: None,
                last_checkin_message: None,
                points: None,
                enabled: true,
                desktop_user_id: Some(cred.user_id.clone()),
                encrypted_credential: Some(encrypted),
                credential_status: Some(status.to_string()),
                data_dir: Some(data_dir.clone()),
                machine_id,
                expires_at: Some(cred.expires_at),
                refresh_expires_at: Some(cred.refresh_expires_at),
                source: AccountSource::Desktop,
                ..Default::default()
            };
            data.upsert_desktop_account(account)
        }
    };
    data.save(&state.path)?;
    drop(data);
    let _ = crate::rebuild_tray_menu(&app);
    Ok(saved.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "trae_mate_a7_relocate_{}_{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_storage(dir: &Path, content: &str) {
        let folder = dir.join("User").join("globalStorage");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("storage.json"), content).unwrap();
    }

    fn backups_in(folder: &Path) -> usize {
        fs::read_dir(folder)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("storage.json.bak_")
            })
            .count()
    }

    #[test]
    fn relocate_renames_when_target_missing() {
        let root = case_dir("new");
        let appdata = root.join("appdata");
        fs::create_dir_all(&appdata).unwrap();
        let temp = root.join("TRAE SOLO CN login temp1");
        write_storage(&temp, "new-storage");

        let expected = appdata.join(format!("{}_1001", trae_machine::DATA_DIR_NAME));
        let (target, copied) = relocate_login_dir(&appdata, &temp, "1001", false).unwrap();

        assert_eq!(target, expected, "返回标准目录");
        assert!(copied, "新目录视为已写入凭据");
        assert!(!temp.exists(), "临时目录已改名");
        assert_eq!(
            fs::read_to_string(target.join("User").join("globalStorage").join("storage.json"))
                .unwrap(),
            "new-storage"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relocate_copies_credential_with_backup_when_target_exists() {
        let root = case_dir("exists");
        let appdata = root.join("appdata");
        let target = appdata.join(format!("{}_1002", trae_machine::DATA_DIR_NAME));
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("marker.txt"), "keep").unwrap();
        write_storage(&target, "old-storage");
        let temp = root.join("TRAE SOLO CN login temp2");
        write_storage(&temp, "new-storage");

        let (got, copied) = relocate_login_dir(&appdata, &temp, "1002", false).unwrap();

        assert_eq!(got, target, "返回既有目录而非临时目录");
        assert!(copied, "凭据已覆盖");
        assert_eq!(
            fs::read_to_string(target.join("marker.txt")).unwrap(),
            "keep",
            "既有文件不被删除"
        );
        let storage_dir = target.join("User").join("globalStorage");
        assert_eq!(
            fs::read_to_string(storage_dir.join("storage.json")).unwrap(),
            "new-storage",
            "storage.json 已更新"
        );
        assert_eq!(backups_in(&storage_dir), 1, "生成 storage.json.bak_* 备份");
        assert!(!temp.exists(), "临时目录已清理");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn relocate_keeps_temp_when_target_running() {
        let root = case_dir("running");
        let appdata = root.join("appdata");
        let target = appdata.join(format!("{}_1003", trae_machine::DATA_DIR_NAME));
        fs::create_dir_all(&target).unwrap();
        write_storage(&target, "old-storage");
        let temp = root.join("TRAE SOLO CN login temp3");
        write_storage(&temp, "new-storage");

        let (got, copied) = relocate_login_dir(&appdata, &temp, "1003", true).unwrap();

        assert_eq!(got, target);
        assert!(!copied, "运行中不写入凭据");
        assert!(temp.exists(), "临时目录保留");
        let storage_dir = target.join("User").join("globalStorage");
        assert_eq!(
            fs::read_to_string(storage_dir.join("storage.json")).unwrap(),
            "old-storage",
            "既有凭据未被覆盖"
        );
        assert_eq!(backups_in(&storage_dir), 0, "未产生备份");
        let _ = fs::remove_dir_all(&root);
    }

    /// dataDir 缺失或为空:视为无可用绑定目录,应回落到主目录
    #[test]
    fn bound_dir_usable_is_false_without_data_dir() {
        let none = Account::default();
        assert!(!bound_dir_usable(&none), "无 dataDir");

        let empty = Account {
            data_dir: Some(String::new()),
            ..Default::default()
        };
        assert!(!bound_dir_usable(&empty), "dataDir 为空串");
    }

    /// 目录存在但缺 User\globalStorage\storage.json:该目录未完成登录,不可用
    #[test]
    fn bound_dir_usable_is_false_without_storage_json() {
        let root = case_dir("bound_missing");
        let dir = root.join("TRAE SOLO CN_2001");
        fs::create_dir_all(dir.join("User").join("globalStorage")).unwrap();

        let account = Account {
            data_dir: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        assert!(!bound_dir_usable(&account), "缺 storage.json");

        let _ = fs::remove_dir_all(&root);
    }

    /// 目录含 storage.json:可用,必须优先用该目录启动(含主账号)
    #[test]
    fn bound_dir_usable_is_true_with_storage_json() {
        let root = case_dir("bound_ok");
        let dir = root.join("TRAE SOLO CN_2002");
        write_storage(&dir, "{}");

        let account = Account {
            data_dir: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        assert!(bound_dir_usable(&account), "storage.json 存在即可用");

        let _ = fs::remove_dir_all(&root);
    }
}
