//! 积分查询/历史/每日快照。移植自参考项目 commands/accounts.rs。
//! 依赖请求:POST /trae/api/v2/pay/ide_user_ent_usage 计算剩余积分(含最早过期时间与今日购买)。

use std::path::PathBuf;

use crate::cooldown;
use crate::fs_utils;
use crate::models::{
    Account, CreditsDailyFile, CreditsDailySnapshot, CreditsFile, RemainingCreditsFile,
};

/// 北京时间固定偏移(不依赖 chrono::Local,某些 Windows 环境可能误判时区)
fn cst() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(8 * 3600).unwrap()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 调用 TRAE API 计算剩余积分。
/// 返回 (剩余积分, 最早过期时间(Unix秒), 今日购买获得积分, 已过期未用额度)。
/// 计算:遍历 user_entitlement_pack_list,仅对 quota.credits_limit 存在的包,
/// 剩余 = credits_limit - usage.credits_amount(≥0) 求和;expire_time > now 取最早;
/// start_time 今日本地时间且 charge_amount>0 计"今日购买";expire_time<=now 的剩余额度计入"已过期额度"。
pub async fn calc_remaining_credits(
    jwt: &str,
    client: &reqwest::Client,
) -> Result<(f64, Option<i64>, f64, f64), String> {
    let auth = crate::jwt::normalize_full(jwt);
    let resp = client
        .post("https://api.trae.cn/trae/api/v2/pay/ide_user_ent_usage")
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .header("accept", "*/*")
        .json(&serde_json::json!({ "require_usage": true, "req_source": 2 }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("API 请求失败: {e}"))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))?;

    let packs = body
        .get("user_entitlement_pack_list")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "响应中缺少 user_entitlement_pack_list".to_string())?;

    let now_ts = chrono::Utc::now().timestamp();
    // 今日北京时间范围 [00:00:00 +08:00, 次日 00:00:00 +08:00]
    let today_start = chrono::Utc::now()
        .with_timezone(&cst())
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(cst())
        .unwrap()
        .timestamp();
    let today_end = today_start + 86_400;

    let totals = sum_packs(packs, now_ts, today_start, today_end);
    Ok((
        totals.total,
        totals.earliest_expire,
        totals.today_purchased,
        totals.expired_remaining,
    ))
}

/// 额度包聚合结果。total 语义不变(仍含已过期包),expired_remaining 单独暴露虚账。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PackTotals {
    pub total: f64,
    pub earliest_expire: Option<i64>,
    pub today_purchased: f64,
    pub expired_remaining: f64,
}

/// 遍历额度包求和(纯函数,便于单测):
/// - total = Σ(credits_limit - used).max(0),含已过期包(保持旧语义,避免余额跳变误判消耗)
/// - earliest_expire = 未过期包中最早的 expire_time
/// - today_purchased = start_time 落在 [today_start, today_end) 且 charge_amount>0 的 credits_limit
/// - expired_remaining = Σ 已过期包(expire_time <= now)的剩余额度
pub fn sum_packs(
    packs: &[serde_json::Value],
    now_ts: i64,
    today_start: i64,
    today_end: i64,
) -> PackTotals {
    let mut totals = PackTotals::default();
    let mut total = 0.0f64;
    let mut expired_remaining = 0.0f64;
    for pack in packs {
        let credits_limit = pack
            .get("entitlement_base_info")
            .and_then(|e| e.get("quota"))
            .and_then(|q| q.get("credits_limit"))
            .and_then(|v| v.as_f64());
        let Some(limit) = credits_limit else {
            continue;
        };
        let used = pack
            .get("usage")
            .and_then(|u| u.get("credits_amount"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let remaining = (limit - used).max(0.0);
        total += remaining;

        // expire_time 在 pack 顶层,取最早且未过期的
        let expire = pack.get("expire_time").and_then(|v| v.as_i64());
        if let Some(exp) = expire {
            if exp > now_ts {
                totals.earliest_expire =
                    Some(totals.earliest_expire.map_or(exp, |e| e.min(exp)));
            } else {
                // 已过期:额度仍留在 total 里(TRAE 不扣余额),单独统计为虚账
                expired_remaining += remaining;
            }
        }

        // 今日购买获得的积分:start_time 在今日北京时间范围内且 charge_amount > 0
        // (签到获得的 pack charge_amount=0,不会误判为购买积分)
        let start_time = pack
            .get("entitlement_base_info")
            .and_then(|e| e.get("start_time"))
            .and_then(|v| v.as_i64());
        let charge_amount = pack
            .get("entitlement_base_info")
            .and_then(|e| e.get("charge_amount"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let is_purchased = charge_amount > 0;
        if let Some(st) = start_time {
            if st >= today_start && st < today_end && is_purchased {
                totals.today_purchased += limit;
            }
        }
    }
    totals.total = round2(total);
    totals.today_purchased = round2(totals.today_purchased);
    totals.expired_remaining = round2(expired_remaining);
    totals
}

fn credits_history_path(state: &crate::store::AppState) -> PathBuf {
    state.data_path("credits_history.json")
}

/// 把账号最新积分与本次新增写入 credits_history.json(按日期追加,永久保留)。
pub fn save_credits_history(
    state: &crate::store::AppState,
    user_id: &str,
    credits: i64,
    delta: i64,
) {
    let path = credits_history_path(state);
    let mut file: CreditsFile = fs_utils::read_json(&path);
    let today = fs_utils::today_prefix();
    file.records.push(crate::models::CreditRecord {
        date: today.clone(),
        user_id: user_id.to_string(),
        credits,
        delta,
    });
    // 不再按 90 天裁剪:每天 1 条约 60B,10 年不足 250KB
    let _ = fs_utils::write_json(&path, &file);
}

/// 该账号最近日期的积分记录(同日期取较大值,跨日期取较新日期)。
pub fn latest_credits_of(state: &crate::store::AppState, user_id: &str) -> Option<i64> {
    let file: CreditsFile = fs_utils::read_json(&credits_history_path(state));
    let mut best: Option<(String, i64)> = None;
    for r in &file.records {
        if r.user_id != user_id {
            continue;
        }
        match &best {
            None => best = Some((r.date.clone(), r.credits)),
            Some((d, c)) => {
                if r.date > *d || (r.date == *d && r.credits > *c) {
                    best = Some((r.date.clone(), r.credits));
                }
            }
        }
    }
    best.map(|(_, c)| c)
}

/// 删除账号联动:清理 remaining_credits.json 与 credits_history.json 中该 user_id 的条目。
pub fn remove_account_data(state: &crate::store::AppState, user_id: &str) {
    let rc_path = state.data_path("remaining_credits.json");
    let mut rc: RemainingCreditsFile = fs_utils::read_json(&rc_path);
    if rc.credits.remove(user_id).is_some() || rc.expire_times.remove(user_id).is_some() {
        rc.updated_at = Some(fs_utils::now_iso());
        let _ = fs_utils::write_json(&rc_path, &rc);
    }
    let ch_path = credits_history_path(state);
    let mut ch: CreditsFile = fs_utils::read_json(&ch_path);
    let before = ch.records.len();
    ch.records.retain(|r| r.user_id != user_id);
    if ch.records.len() != before {
        let _ = fs_utils::write_json(&ch_path, &ch);
    }
}

/// 按账号求和算消耗:仅当昨日快照也带 per_account 时启用新算法。
/// 返回 (consumed, consumed_legacy)。
/// 规则:consumed = Σ max(0, 昨日余额 + 今日该号 earned − 今日余额),只看昨日已存在的 uid;
/// 只出现在今日的 uid(新号入库)计 0 消耗;昨日/今日任一缺失的 uid 跳过。
fn compute_consumed(
    prev: Option<&CreditsDailySnapshot>,
    today: &std::collections::BTreeMap<String, f64>,
    earned_by_uid: &std::collections::HashMap<String, f64>,
    merged_total: f64,
    merged_earned: f64,
) -> (f64, bool) {
    let prev_total = prev.map(|p| p.total).unwrap_or(0.0);
    if let Some(prev_map) = prev.and_then(|p| p.per_account.as_ref()) {
        if !prev_map.is_empty() {
            let by_uid = consumed_by_uid(prev_map, today, earned_by_uid);
            return (round2(by_uid.values().sum()), false);
        }
    }
    // 昨日无个体余额(历史数据):退回旧公式,并标记该条由旧算法得出
    (
        round2((merged_total - merged_earned - prev_total).abs()),
        true,
    )
}

/// 逐账号消耗(规则与 compute_consumed 完全一致,供看板明细复用):
/// 只遍历昨日存在的 uid;prev<=0 且 today<=0 跳过;
/// consumed_i = max(0, 昨日余额 + 今日该号入账 − 今日余额),逐号 round2。
pub fn consumed_by_uid(
    prev_per: &std::collections::BTreeMap<String, f64>,
    today_per: &std::collections::BTreeMap<String, f64>,
    earned_by_uid: &std::collections::HashMap<String, f64>,
) -> std::collections::BTreeMap<String, f64> {
    let mut out: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for (uid, prev_i) in prev_per {
        let Some(today_i) = today_per.get(uid) else {
            continue;
        };
        // 余额为 0 的号即使签到记录 delta>0,也没有真实余额流动,不计消耗
        if *prev_i <= 0.0 && *today_i <= 0.0 {
            continue;
        }
        let earned_i = earned_by_uid.get(uid).copied().unwrap_or(0.0);
        out.insert(
            uid.clone(),
            round2((prev_i + earned_i - today_i).max(0.0)),
        );
    }
    out
}

/// 余额观测入账:昨日已存在账号的 max(0, 今日余额 − 昨日余额) 之和。
/// 今日新入库的 uid 不在昨日快照里,自然被排除(新号入库不计入账口径)。
fn earned_observed(
    prev: Option<&CreditsDailySnapshot>,
    today: &std::collections::BTreeMap<String, f64>,
) -> f64 {
    let Some(prev_map) = prev.and_then(|p| p.per_account.as_ref()) else {
        return 0.0;
    };
    let sum: f64 = prev_map
        .iter()
        .filter_map(|(uid, prev_i)| {
            today
                .get(uid.as_str())
                .map(|today_i| (*today_i - *prev_i).max(0.0))
        })
        .sum();
    round2(sum)
}

/// 余额上升的号数:净利润为正的号,其消耗在余额快照上不可观测
fn unobservable_count(
    prev: Option<&CreditsDailySnapshot>,
    today: &std::collections::BTreeMap<String, f64>,
) -> usize {
    let Some(prev_map) = prev.and_then(|p| p.per_account.as_ref()) else {
        return 0;
    };
    prev_map
        .iter()
        .filter(|(uid, prev_i)| {
            today
                .get(uid.as_str())
                .map_or(false, |today_i| *today_i - **prev_i > 0.0)
        })
        .count()
}

/// 记录入账与观测入账的缺口:只暴露正向(观测 > 记录,说明存在本应用未记录的入账)
pub fn earned_gap(recorded: f64, observed: f64) -> f64 {
    round2((observed - recorded).max(0.0))
}

/// 是否保留本地旧值:接口返回 0 而本地已有正值 → 视为疑似取数失败。
/// 真实下降(new > 0 且小于 old)与真实清零(old == 0)都不受影响。
pub fn should_keep_old_value(old: f64, new: f64) -> bool {
    new == 0.0 && old > 0.0
}

/// 当日 per_account:剔除停用账号的 uid(rc.credits 原样保留,不删不改)。
fn enabled_today_map(
    credits: &std::collections::HashMap<String, f64>,
    disabled: &std::collections::HashSet<String>,
) -> std::collections::BTreeMap<String, f64> {
    credits
        .iter()
        .filter(|(uid, _)| !disabled.contains(*uid))
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

/// 快照 total 与 per_account 同源:只用启用号余额合计。
fn snapshot_total(per_account: &std::collections::BTreeMap<String, f64>) -> f64 {
    round2(per_account.values().sum())
}

/// 记录每日积分快照(每天计算一次):total=启用账号余额合计(与 per_account 同源);
/// earned=今日签到获得(delta 之和)+ 购买获得;
/// consumed 优先按账号求和(见 compute_consumed),昨日缺 per_account 时退回旧公式并标 legacy。
/// earned_recorded=earned(本应用记录入账);earned_observed=余额观测入账(新号排除);
/// 两者之差即「不可观测消耗」的正向证据(见 earned_gap);per_account_expire 供到期账使用。
pub fn record_daily_snapshot(
    state: &crate::store::AppState,
    rc: &RemainingCreditsFile,
    non_checkin_earned: f64,
    expired_total: f64,
) {
    let today = fs_utils::today_prefix();

    let mut file: CreditsDailyFile = fs_utils::read_json(&state.data_path("credits_daily.json"));

    let credits_file: CreditsFile = fs_utils::read_json(&credits_history_path(state));
    let mut checkin_earned = 0.0;
    let mut earned_by_uid: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for r in &credits_file.records {
        if r.date != today || r.user_id == "_daily_total" {
            continue;
        }
        let delta = r.delta as f64;
        checkin_earned += delta;
        *earned_by_uid.entry(r.user_id.clone()).or_insert(0.0) += delta;
    }
    let earned = round2(checkin_earned + non_checkin_earned);

    let today_map: std::collections::BTreeMap<String, f64> = {
        // 停用账号不参与统计:从当日 per_account 剔除(旧值仍保留在 remaining_credits.json 里,不删除)
        let disabled: std::collections::HashSet<String> = {
            let data = state.data.lock().unwrap();
            data.get_accounts()
                .iter()
                .filter(|a| !a.enabled)
                .filter_map(|a| crate::store::effective_user_id(a).map(str::to_string))
                .collect()
        };
        enabled_today_map(&rc.credits, &disabled)
    };
    // total 与 per_account 同源:只用启用号合计,避免停用号余额把「当前总余额」抬高到与明细对不上
    let total = snapshot_total(&today_map);
    let prev = file
        .snapshots
        .iter()
        .filter(|s| s.date < today)
        .last()
        .cloned();
    let (consumed, legacy) =
        compute_consumed(prev.as_ref(), &today_map, &earned_by_uid, total, earned);
    let observed = earned_observed(prev.as_ref(), &today_map);
    let unobservable = unobservable_count(prev.as_ref(), &today_map);
    let per_account_expire: std::collections::BTreeMap<String, i64> = rc
        .expire_times
        .iter()
        .filter(|(uid, _)| today_map.contains_key(uid.as_str()))
        .map(|(uid, ts)| (uid.clone(), *ts))
        .collect();

    let snap = CreditsDailySnapshot {
        date: today.clone(),
        total,
        earned,
        consumed,
        per_account: Some(today_map),
        consumed_legacy: legacy,
        earned_recorded: Some(earned),
        earned_observed: Some(observed),
        per_account_expire: if per_account_expire.is_empty() {
            None
        } else {
            Some(per_account_expire)
        },
        expired: Some(round2(expired_total)),
    };
    if let Some(existing) = file.snapshots.iter_mut().find(|s| s.date == today) {
        // 同日重复刷新:整条重写(earned 由当日历史重新求和,不累加)
        *existing = snap;
    } else {
        // 仅当日首次新建快照时记录一次不可观测号数,同日重复刷新不再追写日志
        if unobservable > 0 {
            fs_utils::app_log(
                &state.data_dir,
                &format!(
                    "积分快照: {unobservable} 个账号余额上升,其消耗不可观测(记录入账 {earned:.2} / 观测入账 {observed:.2})"
                ),
            );
        }
        file.snapshots.push(snap);
    }

    // 不再按 90 天裁剪(永久保留)
    let _ = fs_utils::write_json(&state.data_path("credits_daily.json"), &file);
}

/// 解析账号当前可用的完整 JWT(desktop: 实例回读凭据的 token;jwt: 存储的 jwt)。
/// 供积分查询/自动解冻使用,不写入冷却或触发网络刷新。
pub fn resolve_account_jwt(account: &Account) -> Option<String> {
    match account.source {
        crate::models::AccountSource::Jwt => {
            let enc = account.encrypted_jwt.as_deref()?;
            let sec = crate::credentials::decrypt_jwt_secret(enc).ok()?;
            Some(crate::jwt::normalize_full(&sec.jwt))
        }
        crate::models::AccountSource::Desktop => {
            let enc = account.encrypted_credential.as_deref()?;
            let cred = crate::credentials::decrypt_credential(enc).ok()?;
            // 从实例目录回读最新 token(与签到引擎一致的凭据解析路径)
            let (synced, _) = crate::trae_instance::sync_credential_from_instance(account, &cred);
            Some(crate::jwt::normalize_full(&synced.token))
        }
    }
}

/// 刷新所有账号剩余积分(批量请求 API),返回成功数量。
/// 同时执行自动解冻:签到成功且有积分(credits>0)且冷却类型非 SessionDead → 清除冷却。
pub async fn refresh_remaining_credits(
    state: &crate::store::AppState,
    client: &reqwest::Client,
) -> Result<usize, String> {
    let accounts: Vec<Account> = {
        let data = state.data.lock().unwrap();
        data.get_accounts().to_vec()
    };
    let mut rc: RemainingCreditsFile =
        fs_utils::read_json(&state.data_path("remaining_credits.json"));
    let mut ok_count = 0usize;
    let mut thawed_count = 0usize;
    let mut total_non_checkin_earned: f64 = 0.0;
    let mut total_expired: f64 = 0.0;

    for a in &accounts {
        // 停用账号不参与统计:不发请求、不写 rc.credits、不进 per_account
        if !a.enabled {
            continue;
        }
        let Some(uid) = crate::store::effective_user_id(a) else {
            continue;
        };
        let Some(jwt) = resolve_account_jwt(a) else {
            continue;
        };
        match calc_remaining_credits(&jwt, client).await {
            Ok((credits, expire_at, non_checkin_earned, expired_remaining)) => {
                let old = rc.credits.get(uid).copied().unwrap_or(0.0);
                if should_keep_old_value(old, credits) {
                    // 疑似取数失败(会话失效会把余额写成 0):保留旧值,不计成功
                    fs_utils::app_log(
                        &state.data_dir,
                        &format!("疑似取数失败,保留旧值 uid={uid} 旧={old:.2} 新=0"),
                    );
                    continue;
                }
                rc.credits.insert(uid.to_string(), credits);
                if let Some(exp) = expire_at {
                    rc.expire_times.insert(uid.to_string(), exp);
                }
                total_non_checkin_earned += non_checkin_earned;
                total_expired += expired_remaining;
                ok_count += 1;
                // 自动解冻:有积分 + 冷却类型非 SessionDead → 清除
                if credits > 0.0 {
                    if let Some((et, _, _)) = cooldown::cooldown_of(state, uid, chrono::Utc::now().timestamp()) {
                        if et != "SessionDead" {
                            cooldown::clear_cooldown(state, uid);
                            thawed_count += 1;
                            fs_utils::app_log(
                                &state.data_dir,
                                &format!("自动解冻 [{}]: 类型={} 积分={}", a.name, et, credits),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                fs_utils::app_log(
                    &state.data_dir,
                    &format!("获取剩余积分失败 [{}]: {e}", a.name),
                );
            }
        }
    }

    rc.updated_at = Some(fs_utils::now_iso());
    let _ = fs_utils::write_json(&state.data_path("remaining_credits.json"), &rc);
    record_daily_snapshot(state, &rc, total_non_checkin_earned, total_expired);

    if thawed_count > 0 {
        fs_utils::app_log(
            &state.data_dir,
            &format!("积分刷新完成: 成功 {ok_count}, 自动解冻 {thawed_count}"),
        );
    }
    Ok(ok_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    fn balances(items: &[(&str, f64)]) -> BTreeMap<String, f64> {
        items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    /// 两日均有 per_account:一个号降 2000(签到 900 后再消耗)、一个新号入库 2700
    /// 新算法 = 2000;旧净值公式 = 700(5100 与 2700 相互抵消)
    #[test]
    fn consumed_sums_per_account_drops() {
        let prev = CreditsDailySnapshot {
            date: "2026-10-30".into(),
            total: 5000.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("a", 5000.0)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("a", 3900.0), ("b", 2700.0)]);
        let earned_by_uid: HashMap<String, f64> = [("a".to_string(), 900.0)].into_iter().collect();

        let (consumed, legacy) = compute_consumed(Some(&prev), &today, &earned_by_uid, 6600.0, 900.0);
        assert_eq!(consumed, 2000.0);
        assert!(!legacy);

        // 旧公式(净值绝对值)只有 700,新算法必须区分出个体消耗
        assert_eq!(round2((6600.0 - 900.0 - 5000.0_f64).abs()), 700.0);
    }

    /// 昨日缺 per_account(历史数据):退回旧公式,并标记 consumed_legacy = true
    #[test]
    fn consumed_falls_back_to_legacy_when_yesterday_lacks_per_account() {
        let prev = CreditsDailySnapshot {
            date: "2026-10-30".into(),
            total: 5000.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: None,
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("a", 3900.0), ("b", 2700.0)]);

        let (consumed, legacy) =
            compute_consumed(Some(&prev), &today, &HashMap::new(), 6600.0, 900.0);
        assert_eq!(consumed, 700.0);
        assert!(legacy);
    }

    /// 虚账挡板:余额 0 的号即使签到 delta>0,也不产生真实消耗
    #[test]
    fn consumed_skips_zero_balance_ghost_earn() {
        let prev = CreditsDailySnapshot {
            date: "2026-10-30".into(),
            total: 0.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("a", 0.0)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("a", 0.0)]);
        let earned_by_uid: HashMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();

        let (consumed, legacy) = compute_consumed(Some(&prev), &today, &earned_by_uid, 0.0, 100.0);
        assert_eq!(consumed, 0.0);
        assert!(!legacy);
    }

    /// 观测入账只统计昨日已存在的号:今日新入库的 uid 不得计入
    #[test]
    fn earned_observed_excludes_new_accounts() {
        let prev = CreditsDailySnapshot {
            date: "2026-10-31".into(),
            total: 1000.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("old", 1000.0)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("old", 1200.0), ("new", 2900.0)]);

        assert_eq!(
            earned_observed(Some(&prev), &today),
            200.0,
            "新号 2900 入库必须排除,只留旧号 +200"
        );
        assert_eq!(unobservable_count(Some(&prev), &today), 1, "仅余额上升的旧号计入");
    }

    /// 昨日已存在号既签到(记录 100)又外部入账(余额 +600):gap=500,消耗不得为负
    #[test]
    fn earned_gap_exposes_unrecorded_increase_without_negative_consumed() {
        let prev = CreditsDailySnapshot {
            date: "2026-10-31".into(),
            total: 1000.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("a", 1000.0)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("a", 1600.0)]);
        let earned_by_uid: HashMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();

        let (consumed, legacy) =
            compute_consumed(Some(&prev), &today, &earned_by_uid, 1600.0, 100.0);
        assert_eq!(consumed, 0.0, "余额净增时消耗必须钳到 0,不得为负");
        assert!(!legacy);
        assert_eq!(earned_observed(Some(&prev), &today), 600.0);
        assert_eq!(earned_gap(100.0, 600.0), 500.0, "记录 100 与观测 600 之差即缺口");
    }

    /// 无签到记录的正向入账 + 同日无下降:consumed=0,gap>0
    #[test]
    fn earned_gap_positive_when_increase_unrecorded() {
        let prev = CreditsDailySnapshot {
            date: "2026-10-31".into(),
            total: 500.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("a", 500.0)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("a", 800.0)]);

        let (consumed, legacy) =
            compute_consumed(Some(&prev), &today, &HashMap::new(), 800.0, 0.0);
        assert_eq!(consumed, 0.0, "无记录且余额上升,消耗为 0");
        assert!(!legacy);

        let observed = earned_observed(Some(&prev), &today);
        assert_eq!(observed, 300.0);
        assert!(earned_gap(0.0, observed) > 0.0, "未记录的正向入账必须暴露为缺口");
        assert!(earned_gap(0.0, observed) <= observed, "缺口不得超过观测入账");
    }

    /// 可疑 0:本地有正值、接口回 0 才保留;真实下降与真实清零均照常处理
    #[test]
    fn keep_old_value_only_for_suspicious_zero() {
        assert!(
            should_keep_old_value(449.18, 0.0),
            "旧值 > 0 且新值 = 0 必须保留旧值"
        );
        assert!(!should_keep_old_value(0.0, 0.0), "真实清零照常写入");
        assert!(
            !should_keep_old_value(449.18, 21.0),
            "真实下降(new>0)不得被吞掉"
        );
        assert!(!should_keep_old_value(0.0, 100.0), "正常新增不得被拦");
    }

    /// 停用号不再刷新 → 今日 per_account 缺该 uid → compute_consumed 必须跳过
    #[test]
    fn disabled_account_missing_today_produces_no_consumed() {
        let prev = CreditsDailySnapshot {
            date: "2026-11-01".into(),
            total: 449.18,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("a", 449.18)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[]);

        let (consumed, legacy) =
            compute_consumed(Some(&prev), &today, &HashMap::new(), 0.0, 0.0);
        assert_eq!(consumed, 0.0, "缺今日值的 uid 必须跳过,不得按昨日余额算消耗");
        assert!(!legacy);
    }

    /// 可疑 0 被保留后(今日余额与昨日相同),次日快照不得凭空产生消耗
    #[test]
    fn kept_old_value_yields_zero_consumed() {
        let prev = CreditsDailySnapshot {
            date: "2026-11-01".into(),
            total: 449.18,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("a", 449.18)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let today = balances(&[("a", 449.18)]);

        let (consumed, legacy) =
            compute_consumed(Some(&prev), &today, &HashMap::new(), 449.18, 0.0);
        assert_eq!(consumed, 0.0, "余额持平且无入账,消耗必须为 0");
        assert!(!legacy);
    }

    /// 停用号不参与 total 与 per_account:total 必须与明细逐分同源
    #[test]
    fn disabled_account_excluded_from_total_and_per_account() {
        let credits: HashMap<String, f64> = [
            ("off".to_string(), 500.0),
            ("on".to_string(), 1000.0),
        ]
        .into_iter()
        .collect();
        let disabled: std::collections::HashSet<String> =
            std::collections::HashSet::from(["off".to_string()]);

        let today = enabled_today_map(&credits, &disabled);
        assert_eq!(today.len(), 1, "只保留启用号");
        assert_eq!(today.get("on"), Some(&1000.0));
        assert!(today.get("off").is_none(), "停用号必须从 per_account 剔除");
        assert_eq!(
            snapshot_total(&today),
            1000.0,
            "total 必须等于 per_account 各值之和(不含停用号 500)"
        );
        assert_eq!(
            snapshot_total(&today),
            round2(today.values().sum::<f64>()),
            "total 与明细逐分一致"
        );

        let prev = CreditsDailySnapshot {
            date: "2026-11-01".into(),
            total: 1500.0,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(balances(&[("off", 500.0), ("on", 1000.0)])),
            consumed_legacy: false,
            ..Default::default()
        };
        let (consumed, legacy) =
            compute_consumed(Some(&prev), &today, &HashMap::new(), 1000.0, 0.0);
        assert_eq!(consumed, 0.0, "停用号不得产生消耗");
        assert!(!legacy);
    }

    fn pack(limit: f64, used: f64, expire: Option<i64>) -> serde_json::Value {
        let mut v = serde_json::json!({
            "entitlement_base_info": {"quota": {"credits_limit": limit}},
            "usage": {"credits_amount": used}
        });
        if let Some(exp) = expire {
            v["expire_time"] = serde_json::json!(exp);
        }
        v
    }

    /// 3 个包:过期剩 100 / 有效剩 200 / 过期剩 0 → total=300、expired=100、earliest=有效包
    #[test]
    fn sum_packs_reports_expired_remaining_without_changing_total() {
        let packs = vec![
            pack(150.0, 50.0, Some(100)),  // 已过期,剩 100
            pack(300.0, 100.0, Some(2000)), // 有效,剩 200
            pack(80.0, 80.0, Some(300)),   // 已过期,剩 0
        ];
        let t = sum_packs(&packs, 1000, 0, 86_400);
        assert_eq!(t.total, 300.0, "total 仍含过期包:100 + 200 + 0");
        assert_eq!(t.expired_remaining, 100.0, "只有过期且有剩余的包计入虚账");
        assert_eq!(t.earliest_expire, Some(2000), "earliest 只取未过期包");
    }

    /// 无 expire_time 的包 → 不计入 expired
    #[test]
    fn sum_packs_ignores_packs_without_expire_time() {
        let packs = vec![pack(100.0, 0.0, None)];
        let t = sum_packs(&packs, 1000, 0, 86_400);
        assert_eq!(t.total, 100.0);
        assert_eq!(t.expired_remaining, 0.0, "无 expire_time 不视为过期");
        assert_eq!(t.earliest_expire, None);
    }

    /// expire <= now 但剩余为 0 → expired=0
    #[test]
    fn sum_packs_expired_with_zero_remaining_counts_zero() {
        let packs = vec![pack(80.0, 80.0, Some(500))];
        let t = sum_packs(&packs, 1000, 0, 86_400);
        assert_eq!(t.total, 0.0);
        assert_eq!(t.expired_remaining, 0.0, "剩余为 0 的过期包不计虚账");
    }

    /// 逐号消耗:持平为 0、真实下降计 100,且与 compute_consumed 求和一致
    #[test]
    fn consumed_by_uid_reports_per_account_drops() {
        let prev = balances(&[("a", 449.18), ("b", 100.0)]);
        let today = balances(&[("a", 449.18), ("b", 0.0)]);

        let by_uid = consumed_by_uid(&prev, &today, &HashMap::new());
        assert_eq!(by_uid.get("a"), Some(&0.0), "余额持平不产生消耗");
        assert_eq!(by_uid.get("b"), Some(&100.0), "真实下降计为消耗");
        assert_eq!(by_uid.len(), 2);

        let prev_snap = CreditsDailySnapshot {
            date: "2026-11-02".into(),
            total: 549.18,
            earned: 0.0,
            consumed: 0.0,
            per_account: Some(prev),
            consumed_legacy: false,
            ..Default::default()
        };
        let (consumed, legacy) =
            compute_consumed(Some(&prev_snap), &today, &HashMap::new(), 449.18, 0.0);
        assert_eq!(consumed, 100.0, "逐号求和必须等于汇总消耗");
        assert!(!legacy);
    }

    /// 正增量不产生消耗(即使有签到入账,余额净增仍钳到 0)
    #[test]
    fn consumed_by_uid_ignores_positive_gain() {
        let prev = balances(&[("a", 100.0)]);
        let today = balances(&[("a", 700.0)]);
        let earned: HashMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();

        let by_uid = consumed_by_uid(&prev, &today, &earned);
        assert_eq!(by_uid.get("a"), Some(&0.0), "余额净增时消耗必须为 0");
        assert_eq!(by_uid.values().sum::<f64>(), 0.0);
    }
}
