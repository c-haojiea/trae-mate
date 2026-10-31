import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export interface Account {
  id: string
  name: string
  cookie: string
  createdAt: number
  lastCheckinAt?: number
  lastCheckinResult?: 'success' | 'failed' | 'pending'
  lastCheckinMessage?: string
  points?: number
  enabled: boolean
  desktopUserId?: string
  credentialStatus?: 'valid' | 'expiring' | 'expired'
  dataDir?: string
  machineId?: string
  // 令牌到期时间(毫秒时间戳),随凭据持久化,旧账号回读前为 undefined
  expiresAt?: number
  refreshExpiresAt?: number
  // 实例目录是否仍登录(true/false),未绑定目录或读失败为 null/undefined
  instanceLoggedIn?: boolean | null
  // 参考模式新增
  source?: 'desktop' | 'jwt'
  userId?: string
  jwtExpTimestamp?: number
  jwtStatus?: string // ok | warn | expired | unknown
  hasRefreshToken?: boolean
  jwtAutoRefresh?: boolean
  remainingCredits?: number
  creditsExpireAt?: number
  deviceIdMasked?: string
  cooldownType?: string
  cooldownUntil?: number
  cooldownReason?: string
  checkedToday?: boolean
}

export interface CheckinLog {
  id: string
  accountId: string
  accountName: string
  time: number
  result: 'success' | 'failed'
  message: string
  pointsGained?: number
}

export interface CheckinResult {
  success: boolean
  message: string
  points?: number
}

export interface PointsResult {
  success: boolean
  message: string
  totalPoints?: number
}

export interface CheckinProgressEvent {
  type: string
  index?: number
  total?: number
  user_id?: string
  name?: string
  status?: string // success | fail | already | skip
  message?: string
  code?: number | null
  credits?: number | null
  delta?: number | null
  error_type?: string
  cooldown_until?: number
}

export interface CreditsDailySnapshot {
  date: string
  total: number
  earned: number
  consumed: number
}

export interface JwtPreview {
  userId?: string
  expHours?: number
  expTimestamp?: number
}

export interface LaunchResult {
  dataDir: string
  machineId: string
  launched: boolean
}

export interface InstanceState {
  running: boolean
  source: 'none' | 'main' | 'tool'
  isMainAccount: boolean
}

export interface InstanceDirInfo {
  dataDir: string
  userId: string
  accountName: string
  /** token 过期时间(毫秒),0 表示未知 */
  expiresAt: number
  /** 含设备签名密钥(可刷新 token);false 则 token 过期后需重新登录 */
  hasSigningKey: boolean
  running: boolean
  /** 已被应用内账号绑定(同 dataDir 或同 userId) */
  bound: boolean
}

export interface AppSettings {
  autoCheckin: boolean
  checkinTime: string
  retryCount: number
  retryDelay: number
  /** 自动签到(定时)时账号间执行间隔(分钟),最短 3 */
  autoCheckinIntervalMin?: number
  notifyOnSuccess: boolean
  notifyOnFailed: boolean
  launchAtLogin: boolean
  // 以下字段为兼容旧 UI(已隐藏),desktop 模式下后端不再使用
  checkinMode?: 'webview' | 'api'
  apiConfig?: {
    checkinUrl: string
    method: 'GET' | 'POST'
    headers: Record<string, string>
    body?: Record<string, any>
  }
}

export const useAppStore = defineStore('app', () => {
  // 状态
  const accounts = ref<Account[]>([])
  const logs = ref<CheckinLog[]>([])
  const settings = ref<AppSettings | null>(null)
  const loading = ref(false)
  const checkingIn = ref(false)
  const launching = ref(false) // 多开启动中
  const waitingLogin = ref(false) // 等待新实例登录导入(打开新实例登录后置 true,login-imported 事件后置 false)
  const instanceRefreshTick = ref(0) // 多开状态刷新信号:自增触发各卡片重查
  const nextRunTime = ref<string | null>(null)
  const checkinProgress = ref<Record<string, CheckinProgressEvent>>({}) // 一键签到实时进度(按 user_id 索引)
  const checkinTotal = ref(0)
  const creditsDaily = ref<CreditsDailySnapshot[]>([])
  // remaining_credits.json 的 updated_at(数据截至时间)
  const creditsUpdatedAt = ref<string | null>(null)

  // 计算属性
  const enabledAccounts = computed(() => accounts.value.filter(a => a.enabled))
  const todayCheckedCount = computed(() => {
    // 优先参考模式的 checkedToday,回退旧 lastCheckinAt 判断
    const checked = accounts.value.filter(a => a.checkedToday === true).length
    if (checked > 0) return checked
    const today = new Date().toDateString()
    return accounts.value.filter(a => {
      if (!a.lastCheckinAt) return false
      return new Date(a.lastCheckinAt).toDateString() === today && a.lastCheckinResult === 'success'
    }).length
  })
  const progressDone = computed(() => Object.keys(checkinProgress.value).length)

  // 方法
  async function fetchAccounts() {
    try {
      const result = await invoke<Account[]>('get_accounts')
      accounts.value = result || []
    } catch (e) {
      console.error('获取账号列表失败:', e)
    }
  }

  async function importDesktopAccount() {
    const result = await invoke('import_desktop_account')
    await fetchAccounts()
    return result
  }

  async function updateAccount(id: string, updates: Partial<Account>) {
    try {
      const result = await invoke('update_account', { id, updates })
      await fetchAccounts()
      return result
    } catch (e) {
      console.error('更新账号失败:', e)
      throw e
    }
  }

  async function deleteAccount(id: string) {
    try {
      await invoke('delete_account', { id })
      await fetchAccounts()
    } catch (e) {
      console.error('删除账号失败:', e)
      throw e
    }
  }

  // 在途签到的账号 id(防连点重复请求触发服务端限频)
  const checkingIds = new Set<string>()

  async function checkinAccount(id: string) {
    if (checkingIds.has(id)) {
      return { success: false, message: '该账号签到进行中，请稍候' } as CheckinResult
    }
    // 一键签到进行中不接受单账号签到,避免并发请求触发限频
    if (checkingIn.value) {
      return { success: false, message: '一键签到进行中，请稍候' } as CheckinResult
    }
    checkingIds.add(id)
    try {
      checkingIn.value = true
      const result = await invoke<CheckinResult>('checkin_account', { id })
      await fetchAccounts()
      await fetchLogs()
      return result
    } finally {
      checkingIn.value = false
      checkingIds.delete(id)
    }
  }

  async function checkinAll() {
    try {
      checkingIn.value = true
      const result = await invoke<Array<[Account, CheckinResult]>>('checkin_all')
      await fetchAccounts()
      await fetchLogs()
      return result
    } finally {
      checkingIn.value = false
    }
  }

  async function getAccountPoints(id: string) {
    try {
      const result = await invoke<PointsResult>('get_account_points', { id })
      await fetchAccounts()
      return result
    } catch (e) {
      console.error('获取积分失败:', e)
      throw e
    }
  }

  async function fetchLogs(limit = 100) {
    try {
      const result = await invoke<CheckinLog[]>('get_logs', { limit })
      logs.value = result || []
    } catch (e) {
      console.error('获取日志失败:', e)
    }
  }

  async function clearLogs() {
    try {
      await invoke('clear_logs')
      await fetchLogs()
    } catch (e) {
      console.error('清空日志失败:', e)
      throw e
    }
  }

  async function fetchSettings() {
    try {
      const result = await invoke<AppSettings>('get_settings')
      settings.value = result
    } catch (e) {
      console.error('获取设置失败:', e)
    }
  }

  async function saveSettings(newSettings: Partial<AppSettings>) {
    try {
      const result = await invoke<AppSettings>('save_settings', { settings: newSettings })
      settings.value = result
      await fetchNextRunTime()
      return result
    } catch (e) {
      console.error('保存设置失败:', e)
      throw e
    }
  }

  async function fetchNextRunTime() {
    try {
      const result = await invoke<string | null>('get_next_run_time')
      nextRunTime.value = result
    } catch (e) {
      console.error('获取下次执行时间失败:', e)
    }
  }

  // 多开实例:用账号凭据启动免登录的独立 TRAE 实例
  async function launchMulti(id: string) {
    try {
      launching.value = true
      const result = await invoke<LaunchResult>('launch_account_multi', { id })
      await fetchAccounts()
      return result
    } finally {
      launching.value = false
    }
  }

  // 打开新的空白 TRAE 实例供用户登录(登录后后端自动导入账号并 emit login-imported 事件)
  // waitingLogin 标记等待状态,供界面显示持续提示;login-imported 事件回调中复位
  async function openNewLoginInstance() {
    waitingLogin.value = true
    try {
      await invoke('open_new_login_instance')
    } catch (e) {
      waitingLogin.value = false
      throw e
    }
  }

  // 扫描 %APPDATA% 下已存在的多开/登录临时目录(含登录信息的)
  async function scanInstanceDirs(): Promise<InstanceDirInfo[]> {
    try {
      return await invoke<InstanceDirInfo[]>('scan_instance_dirs')
    } catch (e) {
      console.error('扫描多开目录失败:', e)
      return []
    }
  }

  // 从已有多开目录导入账号(同 userId 已存在则更新凭据并绑定目录)
  async function importAccountFromDir(dataDir: string) {
    const result = await invoke<Account>('import_account_from_dir', { dataDir })
    await fetchAccounts()
    return result
  }

  // 手动刷新账号凭证(实例目录回读 + ExchangeToken 刷新 + 回写)
  async function refreshCredential(id: string) {
    const result = await invoke<Account>('refresh_account_credential', { id })
    await fetchAccounts()
    return result
  }

  // 查询账号实例运行状态(含来源:主实例/工具实例/未运行)
  async function getInstanceState(id: string): Promise<InstanceState> {
    try {
      return await invoke<InstanceState>('get_account_instance_state', { id })
    } catch (e) {
      console.error('查询实例状态失败:', e)
      return { running: false, source: 'none', isMainAccount: false }
    }
  }

  // 触发所有账号卡片重新查询多开实例运行状态(关闭实例后状态不自动更新,手动刷新用)
  function refreshInstances() {
    instanceRefreshTick.value++
  }

  // 聚焦账号多开实例的窗口(提到前台)
  async function focusInstance(id: string) {
    await invoke('focus_account_instance', { id })
  }

  // TRAE 客户端路径
  async function getTraeExePath(): Promise<string | null> {
    try {
      return await invoke<string | null>('get_trae_exe_path')
    } catch (e) {
      console.error('获取 TRAE 路径失败:', e)
      return null
    }
  }

  async function setTraeExePath(path: string) {
    await invoke('set_trae_exe_path', { path })
  }

  async function scanTraeExePath(): Promise<string> {
    return await invoke<string>('scan_trae_exe_path')
  }

  // ===== 参考模式:JWT 账号 / 冷却 / 积分 =====

  // 手动录入 JWT 账号
  async function addJwtAccount(name: string, jwt: string, refreshToken?: string, enabled?: boolean) {
    const result = await invoke('account_add_jwt', { name, jwt, refreshToken, enabled })
    await fetchAccounts()
    return result
  }

  // 解析 JWT 预览(弹窗实时显示 user_id / 剩余小时)
  async function parseJwtPreview(jwt: string): Promise<JwtPreview> {
    return await invoke<JwtPreview>('jwt_parse_preview', { jwt })
  }

  // 手动刷新 jwt 账号(refresh_token -> ExchangeToken)
  async function refreshJwt(userId: string) {
    const result = await invoke('refresh_jwt_account', { userId })
    await fetchAccounts()
    return result
  }

  // 清除单个/全部冷却
  async function clearCooldown(userId: string) {
    await invoke('cooldown_clear', { userId })
    await fetchAccounts()
  }
  async function clearAllCooldowns() {
    await invoke('cooldown_clear_all')
    await fetchAccounts()
  }

  // 实时查询单个/全部账号剩余积分
  async function fetchRemainingCredits(userId: string) {
    const result = await invoke<number>('fetch_remaining_credits', { userId })
    await fetchAccounts()
    return result
  }
  async function refreshAllCredits() {
    const result = await invoke<number>('refresh_all_remaining_credits')
    await fetchAccounts()
    return result
  }

  // 每日积分快照
  async function fetchCreditsDaily() {
    const result = await invoke<{ snapshots: CreditsDailySnapshot[]; dataUpdatedAt: string | null }>('credits_daily_list')
    creditsDaily.value = result?.snapshots || []
    creditsUpdatedAt.value = result?.dataUpdatedAt ?? null
    return result
  }

  // 初始化
  async function init() {
    loading.value = true
    try {
      await Promise.all([
        fetchAccounts(),
        fetchLogs(),
        fetchSettings(),
        fetchNextRunTime(),
        fetchCreditsDaily()
      ])
      // 一键签到实时进度事件
      await listen<CheckinProgressEvent>('checkin-start', (e) => {
        const ev = e.payload
        checkinProgress.value = {}
        checkinTotal.value = ev.total ?? 0
      })
      await listen<CheckinProgressEvent>('checkin-progress', (e) => {
        const ev = e.payload
        if (ev.type === 'account' && ev.user_id) {
          checkinProgress.value = { ...checkinProgress.value, [ev.user_id]: ev }
        }
      })
      await listen<CheckinProgressEvent>('checkin-done', async () => {
        await Promise.all([fetchAccounts(), fetchLogs()])
      })
    } finally {
      loading.value = false
    }
  }

  return {
    // 状态
    accounts,
    logs,
    settings,
    loading,
    checkingIn,
    launching,
    waitingLogin,
    instanceRefreshTick,
    nextRunTime,
    checkinProgress,
    checkinTotal,
    creditsDaily,
    creditsUpdatedAt,
    // 计算属性
    enabledAccounts,
    todayCheckedCount,
    progressDone,
    // 方法
    fetchAccounts,
    importDesktopAccount,
    updateAccount,
    deleteAccount,
    checkinAccount,
    checkinAll,
    getAccountPoints,
    fetchLogs,
    clearLogs,
    fetchSettings,
    saveSettings,
    fetchNextRunTime,
    launchMulti,
    openNewLoginInstance,
    scanInstanceDirs,
    importAccountFromDir,
    refreshCredential,
    getInstanceState,
    focusInstance,
    refreshInstances,
    getTraeExePath,
    setTraeExePath,
    scanTraeExePath,
    addJwtAccount,
    parseJwtPreview,
    refreshJwt,
    clearCooldown,
    clearAllCooldowns,
    fetchRemainingCredits,
    refreshAllCredits,
    fetchCreditsDaily,
    init
  }
})
