<template>
  <section class="credits-board card" :class="{ 'credits-page': asPage }">
    <div
      class="cb-header"
      role="button"
      tabindex="0"
      @click="!asPage && (expanded = !expanded)"
      @keydown.enter="!asPage && (expanded = !expanded)"
    >
      <span class="cb-caret" v-if="!asPage">{{ expanded ? '▾' : '▸' }}</span>
      <h2 class="cb-title">积分看板</h2>
      <span class="cb-summary">
        {{ todaySnap ? `今日消耗 ${fmt(todaySnap.consumed)} · 新增 ${fmt(todaySnap.earned)}` : '今日尚未刷新积分' }}
      </span>
      <button
        class="cb-refresh"
        :disabled="exporting"
        title="导出积分日汇总与账号明细 CSV"
        @click.stop="handleExport"
      >
        {{ exporting ? '导出中…' : '导出 CSV' }}
      </button>
      <button
        class="cb-refresh"
        :disabled="refreshing"
        title="刷新所有账号剩余积分"
        @click.stop="handleRefresh"
      >
        {{ refreshing ? '刷新中…' : '刷新积分' }}
      </button>
    </div>

    <div v-if="expanded || asPage" class="cb-body">
      <!-- 今日三项 -->
      <div class="cb-today-head">
        <span class="cb-today-title">今日</span>
        <span class="cb-updated" :class="{ stale: updatedStale }">
          数据截至 {{ updatedText }}<template v-if="updatedStale">（点刷新可更新）</template>
        </span>
      </div>
      <div class="cb-today">
        <div class="cb-stat">
          <span class="cb-stat-label">今日消耗</span>
          <span class="cb-stat-value">
            {{ todaySnap ? fmt(todaySnap.consumed) : '—' }}
            <span
              v-if="todaySnap && todaySnap.consumed_legacy"
              class="cb-legacy"
              title="该条由旧算法得出：consumed=|今日总额−新增−昨日总额| 取净值绝对值，会把新号入库/签到与消耗互相抵消，不代表真实消耗"
            >（旧算法·净值）</span>
          </span>
        </div>
        <div class="cb-stat">
          <span class="cb-stat-label">今日新增</span>
          <span class="cb-stat-value success">{{ todaySnap ? fmt(todaySnap.earned) : '—' }}</span>
        </div>
        <div class="cb-stat">
          <span class="cb-stat-label">当前总余额</span>
          <span class="cb-stat-value">{{ todaySnap ? fmt(todaySnap.total) : '—' }}</span>
        </div>
      </div>
      <p v-if="todayGap > 0" class="cb-hint">
        未计入口径的正向入账 {{ fmt(todayGap) }}
        <span
          class="cb-legacy"
          title="earned 只统计本应用签到/购买；此项为余额观测入账与记录之差，可能对应客户端内领取、重登回填、退款"
        >ⓘ</span>
      </p>
      <p v-if="todayExpired > 0" class="cb-hint">
        含已过期额度 {{ fmt(todayExpired) }}
        <span
          class="cb-legacy"
          title="该额度已过期但 TRAE 仍将其留在额度列表并计入余额"
        >ⓘ</span>
      </p>
      <!-- 按账号消耗(今日) -->
      <div class="cb-consumed">
        <div class="cb-consumed-head">
          <span class="cb-consumed-title">按账号消耗（今日）</span>
          <button
            v-if="consumedRows.length > 5"
            class="cb-range-btn"
            @click="showAllConsumed = !showAllConsumed"
          >
            {{ showAllConsumed ? '收起' : '展开全部' }}
          </button>
        </div>
        <ul v-if="consumedRows.length" class="cb-accounts">
          <li v-for="r in visibleConsumedRows" :key="r.uid" class="cb-account-row">
            <span class="cb-account-name">{{ r.name }}</span>
            <span class="cb-account-value">{{ fmt(r.value) }}</span>
          </li>
        </ul>
        <p v-else class="cb-hint">今日无可见消耗（或数据未刷新）</p>
      </div>
      <p v-if="!todaySnap" class="cb-hint">今日尚未刷新积分</p>
      <p v-if="exportPaths.length" class="cb-export-paths">
        已导出：<br />{{ exportPaths[0] }}<br />{{ exportPaths[1] }}
      </p>

      <!-- 近 N 天 -->
      <div class="cb-section">
        <div class="cb-section-head">
          <h3 class="cb-section-title">{{ rangeLabel }}</h3>
          <div class="cb-ranges">
            <button
              v-for="opt in rangeOptions"
              :key="opt.value"
              class="cb-range-btn"
              :class="{ active: range === opt.value }"
              @click="range = opt.value"
            >
              {{ opt.label }}
            </button>
          </div>
        </div>
        <div class="cb-table-wrap">
          <table class="cb-table">
            <thead>
              <tr>
                <th>日期</th>
                <th class="num">新增</th>
                <th class="num">消耗</th>
                <th class="num">余额</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="s in recent" :key="s.date">
                <td>{{ s.date }}</td>
                <td class="num">{{ fmt(s.earned) }}</td>
                <td class="num">
                  {{ fmt(s.consumed) }}<span
                    v-if="s.consumed_legacy"
                    class="cb-legacy-star"
                    title="旧算法·净值口径,不代表真实消耗"
                  >*</span>
                </td>
                <td class="num">{{ fmt(s.total) }}</td>
              </tr>
              <tr v-if="recent.length === 0">
                <td colspan="4" class="cb-empty">暂无快照</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-if="hasLegacyRow" class="cb-footnote">
          * 该日由旧算法得出(|今日总额−新增−昨日总额|),新号入库与消耗会被互相抵消,不代表真实消耗
        </p>
        <p v-if="todayGap > 0" class="cb-footnote">
          今日存在未计入口径的正向入账,消耗可能被低估
        </p>
      </div>

      <!-- 历史累计 -->
      <div class="cb-section cb-history">
        <h3 class="cb-section-title">历史累计</h3>
        <template v-if="store.creditsDaily.length">
          <p class="cb-history-total">
            累计消耗 {{ fmt(historyTotal) }}（{{ historyRangeText }}）
            <span v-if="historyHasLegacy" class="cb-legacy">早期数据含旧算法，仅供参考</span>
          </p>
          <p class="cb-hint">更早数据不可得</p>
          <ul class="cb-accounts">
            <li v-for="m in visibleMonths" :key="m.month" class="cb-account-row">
              <span class="cb-account-name">{{ m.month }}</span>
              <span class="cb-account-value">{{ fmt(m.consumed) }}</span>
              <span class="cb-month-days">
                {{ m.days }} 天<template v-if="m.hasLegacy"> · 含旧算法日</template>
              </span>
            </li>
          </ul>
          <button
            v-if="months.length > 2"
            class="btn btn-sm btn-outline cb-months-toggle"
            @click="showAllMonths = !showAllMonths"
          >
            {{ showAllMonths ? '收起' : '展开全部' }}
          </button>
        </template>
        <template v-else>
          <p class="cb-history-total">累计消耗 0.00</p>
          <p class="cb-hint">暂无历史数据</p>
        </template>
      </div>

      <!-- 每账号当前余额 -->
      <div class="cb-section">
        <h3 class="cb-section-title">账号余额</h3>
        <ul class="cb-accounts">
          <li v-for="a in balances" :key="a.id" class="cb-account-row">
            <span class="cb-account-name">{{ a.name }}</span>
            <span class="cb-account-value">{{ a.remainingCredits != null ? fmt(a.remainingCredits) : '—' }}</span>
          </li>
          <li v-if="balances.length === 0" class="cb-empty">暂无账号</li>
        </ul>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAppStore } from '../stores/app'

const props = defineProps<{
  /// 作为独立页面时:强制展开、不可折叠
  asPage?: boolean
}>()
const store = useAppStore()
const emit = defineEmits<{ (e: 'notify', message: string, type?: 'success' | 'error'): void }>()
const expanded = ref(props.asPage === true)
const refreshing = ref(false)
const exporting = ref(false)
const exportPaths = ref<string[]>([])

// 时间范围档位:7 / 30 / 全部(0 = 不限条数,后端已永久保留)
const range = ref(7)
const rangeOptions = [
  { label: '近 7 天', value: 7 },
  { label: '近 30 天', value: 30 },
  { label: '全部', value: 0 }
]
const rangeLabel = computed(() => {
  if (range.value === 7) return '近 7 天'
  if (range.value === 30) return '近 30 天'
  return '全部'
})

// 北京时间今天,格式 YYYY-MM-DD(与后端 credits_daily.json 的 date 对齐)
const todayStr = computed(() =>
  new Date().toLocaleDateString('en-CA', { timeZone: 'Asia/Shanghai' })
)

const todaySnap = computed(() =>
  store.creditsDaily.find((s: { date: string }) => s.date === todayStr.value)
)

// 未计入口径的正向入账 = 观测入账 − 本应用记录入账(仅新快照带这两个字段,旧快照按 0)
type GapFields = { earned_recorded?: number | null; earned_observed?: number | null }
const todayGap = computed(() => {
  const s = todaySnap.value as unknown as GapFields | undefined
  const recorded = s?.earned_recorded
  const observed = s?.earned_observed
  if (recorded == null || observed == null) return 0
  return Math.max(0, observed - recorded)
})

// 已过期未用额度(虚账):旧快照无此字段,按 0
const todayExpired = computed(() => {
  const s = todaySnap.value as unknown as { expired?: number | null } | undefined
  return s?.expired ?? 0
})

// 按账号消耗(今日):uid -> 消耗,降序,只保留 >0 的号
const showAllConsumed = ref(false)
const nameByUid = computed(() => {
  const map: Record<string, string> = {}
  for (const a of store.accounts) {
    if (a.desktopUserId) map[a.desktopUserId] = a.name
    if (a.userId) map[a.userId] = a.name
  }
  return map
})
const consumedRows = computed(() => {
  const src = (store.consumedByUidToday || {}) as Record<string, number>
  return Object.entries(src)
    .map(([uid, v]) => ({ uid, value: Number(v) || 0, name: nameByUid.value[uid] || uid.slice(-6) }))
    .filter((r) => r.value > 0)
    .sort((a, b) => b.value - a.value)
})
const visibleConsumedRows = computed(() =>
  showAllConsumed.value ? consumedRows.value : consumedRows.value.slice(0, 5)
)

// 历史累计:全部来自 store.creditsDaily(缺 consumed 的行跳过)
const historyTotal = computed(() =>
  store.creditsDaily.reduce((sum, s) => sum + (typeof s.consumed === 'number' ? s.consumed : 0), 0)
)
const historyHasLegacy = computed(() => store.creditsDaily.some((s) => s.consumed_legacy === true))
const historyRangeText = computed(() => {
  const dates = store.creditsDaily.map((s) => s.date).sort()
  if (!dates.length) return '—'
  return `${dates[0]} 至 ${dates[dates.length - 1]}`
})
const months = computed(() => {
  const map = new Map<string, { consumed: number; days: number; hasLegacy: boolean }>()
  for (const s of store.creditsDaily) {
    if (typeof s.consumed !== 'number') continue
    const key = s.date.slice(0, 7)
    const cur = map.get(key) || { consumed: 0, days: 0, hasLegacy: false }
    cur.consumed += s.consumed
    cur.days += 1
    if (s.consumed_legacy === true) cur.hasLegacy = true
    map.set(key, cur)
  }
  return [...map.entries()]
    .map(([month, v]) => ({ month, ...v }))
    .sort((a, b) => b.month.localeCompare(a.month))
})
const showAllMonths = ref(false)
const visibleMonths = computed(() =>
  showAllMonths.value ? months.value : months.value.slice(0, 2)
)

// 按选中档位切片,按日期倒序(0=全部,不截断)
const recent = computed(() => {
  const sorted = [...store.creditsDaily].sort((a, b) => b.date.localeCompare(a.date))
  return range.value === 0 ? sorted : sorted.slice(0, range.value)
})

// 当前表格内是否含旧算法行(决定脚注是否显示)
const hasLegacyRow = computed(() => recent.value.some((s) => s.consumed_legacy))

// 每账号当前余额,降序(未知余额排最后)
const balances = computed(() =>
  [...store.accounts].sort((a, b) => (b.remainingCredits ?? -1) - (a.remainingCredits ?? -1))
)

// 数据截至时间(来源 remaining_credits.json updated_at,形如 2026-10-31T20:54:07)
const updatedAtDate = computed(() => {
  const s = store.creditsUpdatedAt
  if (!s) return null
  const d = new Date(s)
  return isNaN(d.getTime()) ? null : d
})

const updatedText = computed(() => {
  const d = updatedAtDate.value
  if (!d) return '—'
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
})

// 距今超过 30 分钟 -> 橙色提示
const updatedStale = computed(() => {
  const d = updatedAtDate.value
  if (!d) return false
  return Date.now() - d.getTime() > 30 * 60 * 1000
})

async function handleRefresh() {
  if (refreshing.value) return
  refreshing.value = true
  try {
    // refreshAllCredits 内部已 await fetchAccounts() + fetchCreditsDaily(),此处不再重复
    await store.refreshAllCredits()
  } finally {
    refreshing.value = false
  }
}

async function handleExport() {
  if (exporting.value) return
  exporting.value = true
  try {
    const paths = await store.exportCreditsCsv()
    exportPaths.value = paths
    emit('notify', 'CSV 已导出到 data\\exports', 'success')
  } catch (e: any) {
    emit('notify', '导出失败: ' + (e?.message || e), 'error')
  } finally {
    exporting.value = false
  }
}

function fmt(n: number): string {
  return n.toFixed(2)
}
</script>

<style scoped>
.credits-board {
  padding: 0;
  overflow: hidden;
}

.cb-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 18px;
  cursor: pointer;
  user-select: none;
}

.cb-caret {
  color: var(--text-muted);
  font-size: 12px;
  width: 12px;
  flex-shrink: 0;
}

.cb-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-primary);
  flex-shrink: 0;
}

.cb-summary {
  font-size: 12px;
  color: var(--text-secondary);
  margin-left: auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cb-refresh {
  flex-shrink: 0;
  font-size: 12px;
  padding: 4px 10px;
  border-radius: var(--r-sm);
  border: 1px solid var(--glass-border);
  background: var(--surface);
  color: var(--text-primary);
  cursor: pointer;
}

.cb-refresh:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.cb-body {
  padding: 0 18px 16px;
}

.cb-today-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-bottom: 8px;
}

.cb-today-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-secondary);
}

.cb-updated {
  font-size: 11px;
  color: var(--text-muted);
}

.cb-updated.stale {
  color: var(--warning);
}

.cb-today {
  display: flex;
  gap: 10px;
  margin-bottom: 4px;
}

.cb-stat {
  flex: 1;
  min-width: 0;
  padding: 10px 12px;
  border-radius: var(--r-md);
  background: var(--surface);
  border: 1px solid var(--glass-border);
}

.cb-stat-label {
  display: block;
  font-size: 11px;
  color: var(--text-secondary);
  margin-bottom: 4px;
}

.cb-stat-value {
  display: block;
  font-size: 16px;
  font-weight: 700;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cb-stat-value.success {
  color: var(--success);
}

.cb-hint {
  font-size: 12px;
  color: var(--text-muted);
  margin: 6px 0 0;
}

.cb-legacy,
.cb-legacy-star {
  font-size: 11px;
  font-weight: 400;
  color: var(--warning);
}

.cb-footnote {
  font-size: 11px;
  color: var(--text-muted);
  margin: 6px 0 0;
  line-height: 1.5;
}

.cb-export-paths {
  font-size: 11px;
  color: var(--text-muted);
  margin: 6px 0 0;
  line-height: 1.5;
  word-break: break-all;
}

.cb-section {
  margin-top: 14px;
}

.cb-section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 8px;
}

.cb-section-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-secondary);
  margin: 0;
}

.cb-ranges {
  display: flex;
  gap: 6px;
}

.cb-range-btn {
  font-size: 11px;
  padding: 3px 9px;
  border-radius: var(--r-sm);
  border: 1px solid var(--glass-border);
  background: var(--surface);
  color: var(--text-secondary);
  cursor: pointer;
}

.cb-range-btn.active {
  border-color: var(--accent);
  color: var(--accent);
  font-weight: 700;
}

/* 超过 10 行时表格内部滚动,不撑高页面 */
.cb-table-wrap {
  max-height: 262px;
  overflow-y: auto;
}

.cb-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.cb-table th {
  text-align: left;
  font-weight: 600;
  color: var(--text-muted);
  padding: 4px 6px;
  border-bottom: 1px solid var(--glass-border);
}

.cb-table td {
  padding: 5px 6px;
  color: var(--text-primary);
  border-bottom: 1px solid var(--glass-border);
}

.cb-table .num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.cb-accounts {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 220px;
  overflow-y: auto;
}

.cb-account-row {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  padding: 5px 6px;
  font-size: 12px;
  border-bottom: 1px solid var(--glass-border);
}

.cb-account-name {
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cb-account-value {
  color: var(--warning);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.cb-consumed {
  margin: 10px 0 4px;
}

.cb-consumed-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 4px;
}

.cb-consumed-title {
  font-size: 12px;
  color: var(--text-secondary);
}

/* 作为独立页面时:铺满内容区,不折叠 */
.credits-page {
  margin: 0;
}

.credits-page .cb-body {
  padding-top: 4px;
}

.cb-history-total {
  margin: 2px 0 4px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
}

.cb-month-days {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--text-secondary);
}

.cb-months-toggle {
  margin-top: 6px;
}

.cb-empty {
  font-size: 12px;
  color: var(--text-muted);
  padding: 8px 6px;
  text-align: center;
}
</style>
