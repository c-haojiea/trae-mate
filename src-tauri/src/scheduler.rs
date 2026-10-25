// 定时签到调度器。用 generation 计数控制任务生命周期(无需 abort),
// tokio 分段 sleep 以便及时响应 stop。时区 Asia/Shanghai。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Timelike;
use chrono_tz::Asia::Shanghai;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::checkin::perform_all_checkin;
use crate::models::AppSettings;
use crate::store::AppState;

pub struct SchedulerState {
    pub generation: Arc<AtomicU64>,
}

impl Default for SchedulerState {
    fn default() -> Self {
        SchedulerState {
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// 启动定时任务(若已开启自动签到)。每次调用增 generation,旧任务自行退出。
pub fn start_scheduler(app: AppHandle) {
    let gen = app.state::<SchedulerState>().inner().generation.clone();
    let my_gen = gen.fetch_add(1, Ordering::SeqCst) + 1;

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
        // 记录本任务最近一次执行日期,避免同一天反复补签
        let mut last_run_day: Option<chrono::NaiveDate> = None;
        loop {
            if !is_current(&app_clone, my_gen) {
                break;
            }
            // 补签:已过设定时刻(启动晚/休眠错过)且当天仍有未签账号时,立即执行而不等到明天
            if should_catch_up(&app_clone, last_run_day) {
                run_auto_checkin(app_clone.clone()).await;
                last_run_day = Some(today_shanghai());
                continue;
            }
            let dur = match next_run_duration(&app_clone) {
                Some(d) => d,
                None => break,
            };
            // 分段 sleep,每 15s 检查 generation,及时响应 stop/restart
            let mut remaining = dur;
            while remaining > std::time::Duration::ZERO {
                if !is_current(&app_clone, my_gen) {
                    break;
                }
                let step = remaining.min(std::time::Duration::from_secs(15));
                tokio::time::sleep(step).await;
                remaining = remaining.saturating_sub(step);
            }
            if !is_current(&app_clone, my_gen) {
                break;
            }
            run_auto_checkin(app_clone.clone()).await;
            last_run_day = Some(today_shanghai());
        }
    });
}

fn today_shanghai() -> chrono::NaiveDate {
    chrono::Utc::now().with_timezone(&Shanghai).date_naive()
}

/// 是否需要补签:自动签到已开启、已过当天设定时刻、今天还没执行过、且仍有当天未签的启用账号。
fn should_catch_up(app: &AppHandle, last_run_day: Option<chrono::NaiveDate>) -> bool {
    let settings = app
        .state::<AppState>()
        .inner()
        .data
        .lock()
        .unwrap()
        .get_settings();
    if !settings.auto_checkin {
        return false;
    }
    let Some((h, m)) = parse_hhmm(&settings.checkin_time) else {
        return false;
    };
    let now = chrono::Utc::now().with_timezone(&Shanghai);
    if !past_scheduled_time(now, (h, m)) {
        return false; // 还没到设定时刻
    }
    if last_run_day == Some(now.date_naive()) {
        return false; // 今天已执行过
    }
    crate::checkin::has_unchecked_today(app)
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

    fn shanghai_tz() -> chrono_tz::Tz {
        chrono_tz::Asia::Shanghai
    }
}

/// 停止定时任务(增 generation 使当前任务退出)
pub fn stop_scheduler(app: &AppHandle) {
    app.state::<SchedulerState>()
        .inner()
        .generation
        .fetch_add(1, Ordering::SeqCst);
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
    let results = perform_all_checkin(&app, client.inner(), state.inner(), interval_secs, true).await;
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
    let mut parts = s.split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    Some((h, m))
}
