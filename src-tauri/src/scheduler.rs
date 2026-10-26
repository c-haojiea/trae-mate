// 定时签到调度器。用 generation 计数控制任务生命周期(无需 abort),
// tokio 分段 sleep 以便及时响应 stop。时区 Asia/Shanghai。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Timelike;
use chrono_tz::Asia::Shanghai;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::checkin::{perform_all_checkin, sleep_interruptible};
use crate::models::AppSettings;
use crate::store::AppState;

/// 当天补签重试间隔与最大尝试次数
pub(crate) const RETRY_DELAY_SECS: u64 = 30 * 60;
pub(crate) const MAX_ATTEMPTS_PER_DAY: u32 = 3;

pub struct SchedulerState {
    pub generation: Arc<AtomicU64>,
    /// 取消信号:stop 时置 true,批次内逐账号检查以中断
    pub cancel: Arc<AtomicBool>,
    /// 批次互斥:同一时刻只允许一个签到批次执行
    pub running: Arc<AtomicBool>,
}

impl Default for SchedulerState {
    fn default() -> Self {
        SchedulerState {
            generation: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 启动定时任务(若已开启自动签到)。每次调用增 generation,旧任务自行退出。
pub fn start_scheduler(app: AppHandle) {
    let state = app.state::<SchedulerState>();
    let gen = state.inner().generation.clone();
    let cancel = state.inner().cancel.clone();
    let my_gen = gen.fetch_add(1, Ordering::SeqCst) + 1;
    cancel.store(false, Ordering::SeqCst);

    let settings = app
        .state::<AppState>()
        .inner()
        .data
        .lock()
        .unwrap()
        .get_settings();
    if !settings.auto_checkin {
        return;
    }

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        // 当天已执行批次次数与日期:上海日期变化后 attempts 归零
        let mut attempt_day: Option<chrono::NaiveDate> = None;
        let mut attempts: u32 = 0;
        loop {
            if !is_current(&app_clone, my_gen) {
                break;
            }
            let today = today_shanghai();
            if attempt_day != Some(today) {
                attempt_day = Some(today);
                attempts = 0;
            }
            let (past_time, unchecked) = catch_up_state(&app_clone);
            match next_action(past_time, attempts, unchecked) {
                Plan::RunNow => {
                    run_auto_checkin(app_clone.clone()).await;
                    attempts += 1;
                    if cancel.load(Ordering::SeqCst) {
                        break; // 批次被 stop 中断:停止排程
                    }
                }
                Plan::RetryIn(dur) => {
                    if !sleep_interruptible(&cancel, dur).await {
                        break;
                    }
                }
                Plan::SleepToNextDay => {
                    // 签到时间非法(配置损坏)时不退出线程:10 分钟后重试,配置改好后自愈
                    let dur = next_run_duration(&app_clone).unwrap_or(Duration::from_secs(600));
                    if !sleep_interruptible(&cancel, dur).await {
                        break;
                    }
                }
            }
        }
    });
}

/// 主循环动作(纯决策,便于单测)
#[derive(Debug, PartialEq)]
pub(crate) enum Plan {
    RunNow,
    RetryIn(Duration),
    SleepToNextDay,
}

/// 根据当前状态决定下一步:未到点/已用尽次数→睡到明天;
/// 到点且当天未跑→立即执行;已跑但仍有未签→30 分钟后重试。
pub(crate) fn next_action(past_time: bool, attempts: u32, unchecked: bool) -> Plan {
    if !past_time {
        return Plan::SleepToNextDay;
    }
    if attempts == 0 {
        return Plan::RunNow;
    }
    if unchecked && attempts < MAX_ATTEMPTS_PER_DAY {
        return Plan::RetryIn(Duration::from_secs(RETRY_DELAY_SECS));
    }
    Plan::SleepToNextDay
}

/// 补签状态:是否已过当天签到时刻、今天是否仍有未签账号
fn catch_up_state(app: &AppHandle) -> (bool, bool) {
    let settings = app
        .state::<AppState>()
        .inner()
        .data
        .lock()
        .unwrap()
        .get_settings();
    if !settings.auto_checkin {
        return (false, false);
    }
    let Some((h, m)) = parse_hhmm(&settings.checkin_time) else {
        return (false, false);
    };
    let now = chrono::Utc::now().with_timezone(&Shanghai);
    (past_scheduled_time(now, (h, m)), crate::checkin::has_unchecked_today(app))
}

fn today_shanghai() -> chrono::NaiveDate {
    chrono::Utc::now().with_timezone(&Shanghai).date_naive()
}

/// 当前时刻是否已过当天设定时刻(分钟粒度,同一分钟视为已到点)
fn past_scheduled_time(now: chrono::DateTime<chrono_tz::Tz>, (h, m): (u32, u32)) -> bool {
    (now.hour() as u32) * 60 + (now.minute() as u32) >= h * 60 + m
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn past_scheduled_time_boundaries() {
        let tz = shanghai_tz();
        // 2026-10-22 09:00 / 09:14 / 09:15
        let at = |h, m| tz.with_ymd_and_hms(2026, 10, 22, h, m, 0).unwrap();
        assert!(!past_scheduled_time(at(8, 59), (9, 14)));
        assert!(past_scheduled_time(at(9, 14), (9, 14)), "同一分钟视为到点");
        assert!(past_scheduled_time(at(9, 15), (9, 14)));
        assert!(!past_scheduled_time(at(0, 0), (9, 14)));
    }

    #[test]
    fn validate_hhmm_accepts_valid() {
        assert!(validate_hhmm("08:30"));
        assert!(validate_hhmm("00:00"));
        assert!(validate_hhmm("23:59"));
    }

    #[test]
    fn validate_hhmm_rejects_invalid() {
        assert!(!validate_hhmm("8:30"));
        assert!(!validate_hhmm("24:00"));
        assert!(!validate_hhmm("08:60"));
        assert!(!validate_hhmm(""));
    }

    #[test]
    fn parse_hhmm_matches_validation() {
        assert_eq!(parse_hhmm("08:30"), Some((8, 30)));
        assert_eq!(parse_hhmm("8:30"), None);
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("08:60"), None);
        assert_eq!(parse_hhmm(""), None);
    }

    #[test]
    fn next_action_covers_all_plans() {
        assert_eq!(next_action(true, 0, true), Plan::RunNow, "到点未跑立即执行");
        assert_eq!(next_action(true, 1, true), Plan::RetryIn(Duration::from_secs(RETRY_DELAY_SECS)));
        assert_eq!(next_action(true, 2, true), Plan::RetryIn(Duration::from_secs(RETRY_DELAY_SECS)));
        assert_eq!(next_action(true, 3, true), Plan::SleepToNextDay, "达上限不再重试");
        assert_eq!(next_action(true, 1, false), Plan::SleepToNextDay, "无未签账号不再重试");
        assert_eq!(next_action(false, 0, true), Plan::SleepToNextDay, "未到点睡到明天");
    }

    fn shanghai_tz() -> chrono_tz::Tz {
        chrono_tz::Asia::Shanghai
    }
}

/// 停止定时任务(增 generation 使当前任务退出;置 cancel 真正中断进行中的批次)
pub fn stop_scheduler(app: &AppHandle) {
    let state = app.state::<SchedulerState>();
    state.inner().generation.fetch_add(1, Ordering::SeqCst);
    state.inner().cancel.store(true, Ordering::SeqCst);
}

/// 下次执行时间(ISO8601),未启用返回 None
pub fn get_next_run_time(app: &AppHandle) -> Option<String> {
    let settings = app
        .state::<AppState>()
        .inner()
        .data
        .lock()
        .unwrap()
        .get_settings();
    if !settings.auto_checkin {
        return None;
    }
    next_run_instant(&settings).map(|dt| dt.to_rfc3339())
}

fn is_current(app: &AppHandle, gen: u64) -> bool {
    app.state::<SchedulerState>()
        .inner()
        .generation
        .load(Ordering::SeqCst)
        == gen
}

async fn run_auto_checkin(app: AppHandle) {
    let state = app.state::<AppState>();
    let client = app.state::<reqwest::Client>();
    let settings = state.inner().data.lock().unwrap().get_settings();

    // 自动签到账号间冷却间隔(分钟),最短 3 分钟
    let interval_secs = (settings.auto_checkin_interval_min.max(3) as u64) * 60;
    let results = match perform_all_checkin(
        &app,
        client.inner(),
        state.inner(),
        interval_secs,
        true,
    )
    .await
    {
        Ok(r) => r,
        // 已有批次在跑等抢占失败:不通知,交由主循环下一轮处理
        Err(_) => return,
    };
    let success = results.iter().filter(|(_, r)| r.success).count();
    let failed = results.len() - success;

    if settings.notify_on_success && success > 0 {
        let body = format!(
            "成功签到 {} 个账号{}",
            success,
            if failed > 0 {
                format!("，失败 {} 个", failed)
            } else {
                String::new()
            }
        );
        let _ = app
            .notification()
            .builder()
            .title("Trae 签到成功")
            .body(body)
            .show();
    }
    if settings.notify_on_failed && failed > 0 {
        let _ = app
            .notification()
            .builder()
            .title("Trae 签到失败")
            .body(format!("{} 个账号签到失败，请查看日志", failed))
            .show();
    }
}

fn next_run_instant(settings: &AppSettings) -> Option<chrono::DateTime<chrono::Utc>> {
    let (h, m) = parse_hhmm(&settings.checkin_time)?;
    let now = chrono::Utc::now().with_timezone(&Shanghai);
    let today = now.date_naive().and_hms_opt(h, m, 0)?;
    let today = today.and_local_timezone(Shanghai).single()?;
    let next = if today > now {
        today
    } else {
        today + chrono::Duration::days(1)
    };
    Some(next.with_timezone(&chrono::Utc))
}

fn next_run_duration(app: &AppHandle) -> Option<std::time::Duration> {
    let settings = app
        .state::<AppState>()
        .inner()
        .data
        .lock()
        .unwrap()
        .get_settings();
    let next = next_run_instant(&settings)?;
    let now = chrono::Utc::now();
    (next - now).to_std().ok()
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None; // 严格要求两位时/分
    }
    if !bytes[..2].iter().all(|c| c.is_ascii_digit()) || !bytes[3..].iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h: u32 = s[..2].parse().ok()?;
    let m: u32 = s[3..].parse().ok()?;
    if h > 23 || m > 59 {
        return None; // 范围校验
    }
    Some((h, m))
}

/// 严格校验签到时间:HH:MM,两位时/分,00:00-23:59
pub(crate) fn validate_hhmm(s: &str) -> bool {
    parse_hhmm(s).is_some()
}
