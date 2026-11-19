<template>
  <div class="activity-page">
    <!-- 工具条 -->
    <div class="act-toolbar">
      <input v-model="search" class="act-search" type="text" placeholder="搜索账号名、标签或备注" />
      <select v-model="sortBy" class="act-select">
        <option value="recent">最近打开</option>
        <option value="name">名称</option>
        <option value="count">打开次数</option>
      </select>
      <label class="act-check">
        <input v-model="onlyTagged" type="checkbox" />
        <span>仅看有标签</span>
      </label>
      <div class="act-views">
        <button
          v-for="v in views"
          :key="v.key"
          class="act-view-btn"
          :class="{ active: view === v.key }"
          @click="view = v.key"
        >
          {{ v.label }}
        </button>
      </div>
      <button class="btn btn-sm btn-outline" :disabled="loading" @click="reload">
        <Icon name="arrow-path" :size="14" :class="{ spinning: loading }" />
        <span>刷新</span>
      </button>
    </div>

    <!-- 时间窗口筛选(时间线 / 统计共用;账号下拉仅时间线) -->
    <div v-if="view === 'timeline' || view === 'stats'" class="act-filters">
      <div class="act-range">
        <button
          v-for="r in dayRanges"
          :key="r.key"
          class="act-range-btn"
          :class="{ active: days === r.key }"
          @click="days = r.key"
        >
          {{ r.label }}
        </button>
      </div>
      <select v-if="view === 'timeline'" v-model="timelineAccount" class="act-select">
        <option value="">全部账号</option>
        <option v-for="a in timelineAccountOptions" :key="a.id" :value="a.id">
          {{ a.name }}
        </option>
      </select>
      <!-- 统计视图:导出当前窗口的流水 + 汇总 CSV -->
      <button
        v-if="view === 'stats'"
        class="btn btn-sm btn-primary"
        :disabled="exporting"
        @click="doExport"
      >
        {{ exporting ? '导出中…' : '导出 CSV' }}
      </button>
    </div>

    <p class="act-hint">标签为手动维护，用于标记该账号的用途；项目路径自动提取在 SOLO 模式下常为空。历史记录显示的是打开当时的标签。</p>

    <!-- 无打开记录提示(仍保留账号列表以便维护标签) -->
    <div v-if="!loading && store.activityEvents.length === 0" class="act-empty-hint">
      还没有打开记录，从账号卡片启动一次即可产生
    </div>

    <!-- 空态(统计视图自带空态,不套用账号列表空态) -->
    <div v-if="!loading && view !== 'stats' && filtered.length === 0" class="empty-state">
      <Icon name="inbox" :size="40" />
      <p>没有匹配的账号</p>
    </div>

    <!-- 列表视图 -->
    <div v-else-if="view === 'list'" class="act-list">
      <div class="act-list-head">
        <span class="col-name">账号</span>
        <span class="col-tags">标签</span>
        <span class="col-note">备注</span>
        <span class="col-status">状态</span>
        <span class="col-time">最近打开</span>
        <span class="col-count">近30天</span>
        <span class="col-proj">最近项目</span>
      </div>
      <div v-for="a in filtered" :key="a.id" class="act-row">
        <span class="col-name">{{ a.name }}</span>
        <span class="col-tags" @click="openEditor(a)">
          <template v-if="a.tags.length">
            <span v-for="t in a.tags" :key="t" class="act-tag">{{ t }}</span>
          </template>
          <span v-else class="act-tag add">+ 标签</span>
        </span>
        <span class="col-note" :title="a.note || ''">{{ noteBrief(a.note, 30) }}</span>
        <span class="col-status" :class="{ 'status-off': !a.enabled }">{{ statusText(a) }}</span>
        <span class="col-time">{{ fmtRel(a.lastOpenedAt) }}</span>
        <span class="col-count">{{ a.openCount30d }}</span>
        <span class="col-proj" :title="projectTitle(a)">
          <template v-if="a.projects.length">
            {{ baseName(a.projects[0].path) }}
            <span v-if="a.projects.length > 1" class="act-more">+{{ a.projects.length - 1 }}</span>
          </template>
          <span v-else class="act-none">—</span>
        </span>
      </div>
    </div>

    <!-- 卡片视图 -->
    <div v-else-if="view === 'card'" class="act-cards">
      <div
        v-for="a in filtered"
        :key="a.id"
        class="act-card"
        :class="{ 'act-card-highlight': highlightId === a.id }"
        :data-account-id="a.id"
      >
        <div class="act-card-top">
          <span class="act-card-name">{{ a.name }}</span>
          <span class="act-tags" @click="openEditor(a)">
            <template v-if="a.tags.length">
              <span v-for="t in a.tags" :key="t" class="act-tag">{{ t }}</span>
            </template>
            <span v-else class="act-tag add">+ 标签</span>
          </span>
        </div>
        <!-- A63:账号备注摘要(最多 2 行) -->
        <div v-if="a.note" class="act-card-note" :title="a.note">{{ a.note }}</div>
        <div class="act-card-body">
          <div class="act-kv"><span>最近打开</span><b>{{ fmtRel(a.lastOpenedAt) }}</b></div>
          <div class="act-kv"><span>近 30 天</span><b>{{ a.openCount30d }} 次</b></div>
          <div class="act-kv proj">
            <span>项目</span>
            <b v-if="a.projects.length">
              <div v-for="p in a.projects" :key="p.path" class="act-proj-line" :title="p.path">
                {{ baseName(p.path) }}
              </div>
            </b>
            <b v-else class="act-none">—</b>
          </div>
        </div>
        <div class="act-card-foot">
          <span class="badge" :class="a.enabled ? 'badge-success' : 'badge-muted'">
            {{ a.enabled ? '启用' : '停用' }}
          </span>
          <span class="act-foot-item" title="当前余额">余额 {{ fmtBalance(a.balance) }}</span>
          <span class="act-foot-item" :class="expireClass(a.expiresAt)" title="令牌到期">
            令牌 {{ fmtExpire(a.expiresAt) }}
          </span>
        </div>
      </div>
    </div>

    <!-- 统计视图 -->
    <div v-else-if="view === 'stats'" class="act-stats">
      <!-- 顶部三张卡 -->
      <div class="stat-cards">
        <div class="stat-card">
          <span class="stat-label">打开次数</span>
          <b class="stat-value">{{ statTotals.openCount }}</b>
        </div>
        <div class="stat-card">
          <span class="stat-label">已关闭会话</span>
          <b class="stat-value">{{ statTotals.closedCount }}</b>
        </div>
        <div class="stat-card">
          <span class="stat-label">累计使用时长</span>
          <b class="stat-value">{{ fmtDuration(statTotals.totalMs) }}</b>
        </div>
      </div>

      <!-- 每日活跃(最近 7 天) -->
      <div class="stat-section">
        <h4>每日活跃（最近 7 天）</h4>
        <div v-for="d in last7Days" :key="d.date" class="day-row">
          <span class="day-date">{{ d.date.slice(5) }}</span>
          <div class="day-bar-wrap">
            <div class="day-bar opens" :style="{ width: barWidth(d.openCount, maxOpen) }"></div>
          </div>
          <span class="day-num">{{ d.openCount }} 次</span>
          <div class="day-bar-wrap">
            <div class="day-bar time" :style="{ width: barWidth(d.totalMs, maxMs) }"></div>
          </div>
          <span class="day-num">{{ fmtDuration(d.totalMs) }}</span>
        </div>
      </div>

      <!-- 按时长排序的账号 -->
      <div class="stat-section">
        <h4>按时长排序的账号</h4>
        <div v-if="store.usageByAccount.length === 0" class="act-empty-hint">
          选定范围内还没有完整的使用记录（需要先关闭实例）
        </div>
        <template v-else>
          <div
            v-for="u in visibleUsage"
            :key="u.accountId"
            class="usage-row"
            @click="toggleUsage(u.accountId)"
          >
            <span class="usage-name">{{ u.name }}</span>
            <span class="usage-time">{{ fmtDuration(u.totalMs) }}</span>
            <span class="usage-count">{{ u.sessionCount }} 次</span>
            <span class="usage-caret">{{ expandedUsage === u.accountId ? '▾' : '▸' }}</span>
          </div>
          <div v-if="expandedUsage" class="session-list">
            <div v-for="s in expandedSessions" :key="s.at" class="session-row">
              <span>{{ fmtDateTime(s.at) }}</span>
              <span class="session-arrow">→ {{ fmtDateTime(s.closedAt) }}</span>
              <span class="session-dur">{{ fmtDuration(s.durationMs) }}</span>
              <span v-if="s.suspicious" class="session-warn">可疑（超 24h 按 24h 计）</span>
            </div>
          </div>
          <button
            v-if="store.usageByAccount.length > 5"
            class="btn btn-sm btn-outline usage-more"
            @click="showAllUsage = !showAllUsage"
          >
            {{ showAllUsage ? '收起' : `展开全部（${store.usageByAccount.length}）` }}
          </button>
        </template>
      </div>

      <!-- 项目反向查询 -->
      <div class="stat-section">
        <h4>项目反向查询</h4>
        <input
          v-model="projectQuery"
          class="act-search"
          type="text"
          placeholder="输入项目名，查哪些账号打开过"
        />
        <div v-if="projectQuery.trim()" class="proj-results">
          <div v-if="filteredProjects.length === 0" class="act-empty-hint">没有匹配的项目</div>
          <div v-for="p in filteredProjects" :key="p.path" class="proj-row">
            <span class="proj-name" :title="p.path">{{ p.name }}</span>
            <span class="proj-accounts">
              <button
                v-for="id in p.accountIds"
                :key="id"
                class="proj-account-link"
                @click="gotoAccountCard(id)"
              >
                {{ accountName(id) }}
              </button>
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- 时间线视图 -->
    <div v-else class="act-timeline">
      <div v-if="filteredTimeline.length === 0" class="act-empty-hint">该范围内没有打开记录</div>
      <template v-for="g in timelineGroups" :key="g.date">
        <div class="act-day">{{ g.label }}</div>
        <div v-for="(e, i) in g.items" :key="g.date + '-' + i" class="act-event-wrap">
          <div class="act-event">
            <span class="act-event-time">{{ fmtTime(e.at) }}</span>
            <span class="act-event-name">{{ e.name }}</span>
            <span class="act-tags">
              <span v-for="t in e.tags" :key="t" class="act-tag">{{ t }}</span>
            </span>
            <span class="act-event-entry">
              <span>{{ entryLabel(e.entry) }}</span>
              <button class="act-note-btn" @click="openEventNote(e)">
                {{ e.note ? '改备注' : '＋ 备注' }}
              </button>
            </span>
          </div>
          <!-- A63:该次打开的一句话记录 -->
          <div v-if="e.note" class="act-event-note" :title="e.note" @click="openEventNote(e)">
            {{ e.note }}
          </div>
        </div>
      </template>
    </div>

    <!-- 标签编辑弹窗(A63:胶囊输入) -->
    <div v-if="editing" class="act-mask" @click.self="closeEditor">
      <div class="act-editor">
        <h3>{{ editing.name }} · 标签</h3>
        <p class="act-editor-hint">回车或逗号添加，最多 8 个，单个不超过 24 字</p>
        <div class="tag-input-box">
          <span v-for="(t, i) in draftTags" :key="t + i" class="act-tag">
            {{ t }}
            <button type="button" class="tag-x" title="移除" @click.stop="removeDraftTag(i)">×</button>
          </span>
          <input
            ref="editorInput"
            v-model="tagInputText"
            class="tag-input"
            type="text"
            :disabled="draftTags.length >= TAG_MAX"
            :placeholder="draftTags.length >= TAG_MAX ? '已达上限 8 个' : '输入后回车/逗号添加'"
            @keydown="onTagKeydown"
          />
        </div>
        <div class="act-editor-actions">
          <button class="btn btn-sm btn-outline" @click="closeEditor">取消</button>
          <button class="btn btn-sm btn-primary" :disabled="saving" @click="saveTags">保存</button>
        </div>
      </div>
    </div>

    <!-- A63:事件备注编辑弹窗 -->
    <div v-if="eventNoteEditing" class="act-mask" @click.self="closeEventNote">
      <div class="act-editor">
        <h3>{{ eventNoteEditing.name }} · {{ fmtDateTime(eventNoteEditing.at) }} 备注</h3>
        <p class="act-editor-hint">回车保存，最多 500 字；留空保存即清除</p>
        <input
          ref="eventNoteInput"
          v-model="eventNoteText"
          class="act-search"
          type="text"
          maxlength="500"
          placeholder="例如：今天做 Anki 整理"
          @keydown.enter.prevent="saveEventNote"
          @keydown.esc="closeEventNote"
        />
        <div class="act-editor-actions">
          <button class="btn btn-sm btn-outline" @click="closeEventNote">取消</button>
          <button class="btn btn-sm btn-primary" :disabled="eventNoteSaving" @click="saveEventNote">保存</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onActivated, onMounted, ref, watch } from 'vue'
import {
  ACTIVITY_VIEW_KEY,
  useAppStore,
  type ActivityAccount,
  type ActivityTimelineEvent,
} from '../stores/app'
import Icon from './Icon.vue'

const emit = defineEmits(['notify'])
const store = useAppStore()

const loading = ref(false)
const search = ref('')
const sortBy = ref<'recent' | 'name' | 'count'>('recent')
const onlyTagged = ref(false)

const activityViews = ['list', 'card', 'timeline', 'stats'] as const
type ActivityView = (typeof activityViews)[number]

// 视图初值:后端 ui_state → localStorage → 列表(双读,兼容旧数据)
function loadSavedView(): ActivityView {
  const candidates = [store.uiState.activityView, localStorage.getItem(ACTIVITY_VIEW_KEY)]
  for (const c of candidates) {
    if (c && (activityViews as readonly string[]).includes(c)) return c as ActivityView
  }
  return 'list'
}
const view = ref<ActivityView>(loadSavedView())
let viewTouched = false
// 切换视图:后端与 localStorage 双写
watch(view, (v) => {
  viewTouched = true
  localStorage.setItem(ACTIVITY_VIEW_KEY, v)
  void store.saveUiState({ view: v })
})
// 后端偏好可能晚于首次挂载到达:未被用户手动切换前同步一次
watch(
  () => store.uiState.activityView,
  (v) => {
    if (viewTouched) return
    if (v && (activityViews as readonly string[]).includes(v) && v !== view.value) {
      view.value = v as ActivityView
    }
  }
)

const views = [
  { key: 'list' as const, label: '列表' },
  { key: 'card' as const, label: '卡片' },
  { key: 'timeline' as const, label: '时间线' },
  { key: 'stats' as const, label: '统计' },
]

// 统计窗口:7/30/90 天,0 = 全部;默认近 7 天
const dayRanges = [
  { key: 7, label: '近7天' },
  { key: 30, label: '近30天' },
  { key: 90, label: '近90天' },
  { key: 0, label: '全部' },
] as const
const days = ref(7)

async function reload() {
  loading.value = true
  try {
    await store.fetchAccountActivity(days.value)
  } finally {
    loading.value = false
  }
}

// 窗口变化即重新拉取(由后端聚合,前端不重算)
watch(days, () => {
  void reload()
})

onMounted(reload)
// 被 KeepAlive 缓存后,每次切回该页签都要重新拉一次(否则启动实例后仍显示「从未」)
onActivated(reload)

// 三视图共用同一份筛选结果(切换视图不重新请求)
const filtered = computed(() => {
  const kw = search.value.trim().toLowerCase()
  let list = store.activityAccounts.filter((a) => {
    if (onlyTagged.value && a.tags.length === 0) return false
    if (!kw) return true
    return (
      a.name.toLowerCase().includes(kw) ||
      a.tags.some((t) => t.toLowerCase().includes(kw)) ||
      (a.note || '').toLowerCase().includes(kw)
    )
  })
  list = [...list]
  if (sortBy.value === 'name') {
    list.sort((a, b) => a.name.localeCompare(b.name, 'zh-CN'))
  } else if (sortBy.value === 'count') {
    list.sort((a, b) => b.openCount30d - a.openCount30d)
  } else {
    list.sort((a, b) => (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0))
  }
  return list
})

// ===== 时间线筛选(与统计共用同一时间窗口) =====
const timelineAccount = ref('')

// 账号下拉:仅列出有事件的账号,按最近打开倒序
const timelineAccountOptions = computed(() =>
  store.activityAccounts
    .filter((a) => store.activityEvents.some((e) => e.accountId === a.id))
    .slice()
    .sort((a, b) => (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0))
)

const filteredTimeline = computed(() => {
  const cutoff = days.value === 0 ? 0 : Date.now() - days.value * 86_400_000
  return store.activityEvents.filter(
    (e) =>
      e.at >= cutoff && (!timelineAccount.value || e.accountId === timelineAccount.value)
  )
})

// 时间线:按上海自然日分组(后端已按 at 倒序)
const timelineGroups = computed(() => {
  const groups: { date: string; label: string; items: typeof store.activityEvents }[] = []
  for (const e of filteredTimeline.value) {
    const d = new Date(e.at)
    const date = d.toLocaleDateString('en-CA', { timeZone: 'Asia/Shanghai' })
    let g = groups.find((x) => x.date === date)
    if (!g) {
      const today = new Date().toLocaleDateString('en-CA', { timeZone: 'Asia/Shanghai' })
      g = { date, label: date === today ? '今天' : date, items: [] }
      groups.push(g)
    }
    g.items.push(e)
  }
  return groups
})

function fmtRel(ms: number | null) {
  if (!ms) return '从未'
  const diff = Date.now() - ms
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`
  const days = Math.floor(diff / 86_400_000)
  if (days < 30) return `${days} 天前`
  return new Date(ms).toLocaleDateString('en-CA', { timeZone: 'Asia/Shanghai' })
}

function fmtTime(ms: number) {
  return new Date(ms).toLocaleTimeString('zh-CN', {
    timeZone: 'Asia/Shanghai',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function entryLabel(entry: string) {
  if (entry === 'card') return '卡片'
  if (entry === 'tray') return '托盘'
  return '启动'
}

function baseName(p: string) {
  const parts = p.split(/[\\/]/).filter(Boolean)
  return parts.length ? parts[parts.length - 1] : p
}

function projectTitle(a: ActivityAccount) {
  return a.projects.map((p) => p.path).join('\n')
}

// ===== A59:余额 / 令牌到期 / 状态 =====
function fmtBalance(v?: number | null) {
  return v == null ? '—' : v.toFixed(2)
}

function fmtExpire(ms?: number | null) {
  if (!ms) return '—'
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

// <3 天红、<7 天橙,复用现有主题色变量
function expireClass(ms?: number | null) {
  if (!ms) return ''
  const left = ms - Date.now()
  if (left <= 0) return 'text-danger'
  const days = left / 86_400_000
  if (days < 3) return 'text-danger'
  if (days < 7) return 'text-warning'
  return ''
}

// 列表「状态」列:启用/停用 + 令牌剩余天数(如「9.5d」)
function statusText(a: ActivityAccount) {
  const base = a.enabled ? '启用' : '停用'
  if (!a.expiresAt) return `${base} · —`
  const left = a.expiresAt - Date.now()
  if (left <= 0) return `${base} · 已过期`
  return `${base} · ${(left / 86_400_000).toFixed(1)}d`
}

// ===== A62:导出当前窗口的 CSV(流水 + 汇总) =====
const exporting = ref(false)

async function doExport() {
  if (exporting.value) return
  exporting.value = true
  try {
    const paths = await store.exportActivityCsv(days.value)
    const names = paths.map((p) => p.split(/[\\/]/).pop() || p)
    emit('notify', `已导出：${names.join('、')}`, 'success')
  } catch (e: any) {
    emit('notify', String(e?.message || e || '导出失败'), 'error')
  } finally {
    exporting.value = false
  }
}

// ===== 统计视图 =====
const showAllUsage = ref(false)
const expandedUsage = ref<string | null>(null)
const projectQuery = ref('')
const highlightId = ref<string | null>(null)

// 窗口内合计(后端 usageDaily 已按窗口聚合)
const statTotals = computed(() => ({
  openCount: store.usageDaily.reduce((s, d) => s + d.openCount, 0),
  closedCount: store.usageDaily.reduce((s, d) => s + d.closedCount, 0),
  totalMs: store.usageDaily.reduce((s, d) => s + d.totalMs, 0),
}))

// 最近 7 天(无论选定窗口):缺日补 0
const last7Days = computed(() => {
  const map = new Map(store.usageDaily.map((d) => [d.date, d]))
  const out: { date: string; openCount: number; totalMs: number }[] = []
  for (let i = 6; i >= 0; i--) {
    const ms = Date.now() - i * 86_400_000
    const date = new Date(ms).toLocaleDateString('en-CA', { timeZone: 'Asia/Shanghai' })
    const d = map.get(date)
    out.push({ date, openCount: d?.openCount ?? 0, totalMs: d?.totalMs ?? 0 })
  }
  return out
})
const maxOpen = computed(() => Math.max(1, ...last7Days.value.map((d) => d.openCount)))
const maxMs = computed(() => Math.max(1, ...last7Days.value.map((d) => d.totalMs)))

function barWidth(v: number, max: number) {
  return `${Math.round((v / max) * 100)}%`
}

const visibleUsage = computed(() =>
  showAllUsage.value ? store.usageByAccount : store.usageByAccount.slice(0, 5)
)
const expandedSessions = computed(() => {
  const id = expandedUsage.value
  if (!id) return []
  return store.usageByAccount.find((u) => u.accountId === id)?.sessions ?? []
})
function toggleUsage(id: string) {
  expandedUsage.value = expandedUsage.value === id ? null : id
}

// 项目反向查询:不区分大小写,匹配 name 或 path
const filteredProjects = computed(() => {
  const kw = projectQuery.value.trim().toLowerCase()
  if (!kw) return []
  return store.projectIndex.filter(
    (p) => p.name.toLowerCase().includes(kw) || p.path.toLowerCase().includes(kw)
  )
})

function accountName(id: string) {
  return store.activityAccounts.find((a) => a.id === id)?.name || id
}

// 项目结果里点账号名:切到卡片视图并高亮/滚动到该卡
function gotoAccountCard(id: string) {
  view.value = 'card'
  highlightId.value = id
  nextTick(() => {
    const el = document.querySelector(`[data-account-id="${id}"]`)
    el?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  })
}

function fmtDateTime(ms: number) {
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

// 时长格式化:<1 分钟 / N 分钟 / N 小时 M 分
function fmtDuration(ms: number) {
  if (!ms || ms <= 0) return '0 分钟'
  if (ms < 60_000) return '<1 分钟'
  const mins = Math.floor(ms / 60_000)
  if (mins < 60) return `${mins} 分钟`
  const h = Math.floor(mins / 60)
  const m = mins % 60
  return m ? `${h} 小时 ${m} 分` : `${h} 小时`
}

// ===== A63:标签编辑(胶囊输入) =====
const TAG_MAX = 8
const TAG_MAX_LEN = 24
const editing = ref<ActivityAccount | null>(null)
const draftTags = ref<string[]>([])
const tagInputText = ref('')
const saving = ref(false)
const editorInput = ref<HTMLInputElement | null>(null)

function openEditor(a: ActivityAccount) {
  editing.value = { ...a }
  draftTags.value = [...(a.tags || [])]
  tagInputText.value = ''
  nextTick(() => editorInput.value?.focus())
}

function closeEditor() {
  editing.value = null
}

function removeDraftTag(i: number) {
  draftTags.value.splice(i, 1)
}

// 回车/逗号 → 追加胶囊;输入框为空时 Backspace → 删除最后一个
function onTagKeydown(e: KeyboardEvent) {
  if (e.isComposing) return
  if (e.key === 'Enter' || e.key === ',' || e.key === '，') {
    e.preventDefault()
    commitTagInput()
    return
  }
  if (e.key === 'Backspace' && tagInputText.value === '' && draftTags.value.length) {
    e.preventDefault()
    draftTags.value.pop()
  }
}

function commitTagInput() {
  const raw = tagInputText.value.trim()
  tagInputText.value = ''
  if (!raw) return
  for (const part of raw.split(/[,，]/).map((s) => s.trim()).filter(Boolean)) {
    if (draftTags.value.length >= TAG_MAX) {
      emit('notify', `已达上限 ${TAG_MAX} 个`, 'error')
      break
    }
    let t = part
    if (t.length > TAG_MAX_LEN) {
      t = t.slice(0, TAG_MAX_LEN)
      emit('notify', `标签超过 ${TAG_MAX_LEN} 字，已截断`, 'error')
    }
    if (!draftTags.value.includes(t)) draftTags.value.push(t)
  }
}

async function saveTags() {
  if (!editing.value || saving.value) return
  commitTagInput()
  saving.value = true
  try {
    await store.updateAccountTags(editing.value.id, [...draftTags.value])
    emit('notify', '标签已保存', 'success')
    editing.value = null
  } catch (e: any) {
    emit('notify', String(e?.message || e || '保存标签失败'), 'error')
  } finally {
    saving.value = false
  }
}

// ===== A63:备注摘要 / 事件备注 =====
function noteBrief(note: string | null | undefined, max: number) {
  const n = note || ''
  return n.length > max ? n.slice(0, max) + '…' : n || '—'
}

const eventNoteEditing = ref<ActivityTimelineEvent | null>(null)
const eventNoteText = ref('')
const eventNoteSaving = ref(false)
const eventNoteInput = ref<HTMLInputElement | null>(null)

function openEventNote(e: ActivityTimelineEvent) {
  eventNoteEditing.value = e
  eventNoteText.value = e.note || ''
  nextTick(() => eventNoteInput.value?.focus())
}

function closeEventNote() {
  eventNoteEditing.value = null
}

async function saveEventNote() {
  if (!eventNoteEditing.value || eventNoteSaving.value) return
  const e = eventNoteEditing.value
  eventNoteSaving.value = true
  try {
    await store.setActivityNote(e.accountId, e.at, eventNoteText.value.trim() || null)
    emit('notify', '备注已保存', 'success')
    eventNoteEditing.value = null
  } catch (err: any) {
    emit('notify', String(err?.message || err || '保存备注失败'), 'error')
  } finally {
    eventNoteSaving.value = false
  }
}
</script>

<style scoped>
.activity-page {
  padding: 4px 2px 24px;
}

.act-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}

.act-search {
  flex: 1 1 200px;
  min-width: 160px;
  padding: 7px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: var(--bg-secondary);
  color: var(--text-primary);
  font-size: 13px;
}

.act-select {
  padding: 7px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  background: var(--bg-secondary);
  color: var(--text-primary);
  font-size: 13px;
}

/* 时间线筛选条 */
.act-filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}

.act-range {
  display: inline-flex;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
}

.act-range-btn {
  padding: 6px 12px;
  font-size: 12px;
  background: transparent;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.act-range-btn.active {
  background: var(--accent-color, #3b82f6);
  color: #fff;
}

.act-check {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 13px;
  color: var(--text-secondary);
  cursor: pointer;
}

.act-views {
  display: inline-flex;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
}

.act-view-btn {
  padding: 6px 12px;
  font-size: 13px;
  background: transparent;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.act-view-btn.active {
  background: var(--accent-color, #3b82f6);
  color: #fff;
}

.act-hint {
  margin: 0 0 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.act-empty-hint {
  margin-bottom: 12px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px dashed var(--border-color);
  background: var(--bg-secondary);
  font-size: 13px;
  color: var(--text-secondary);
}

.act-list {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  overflow: hidden;
}

.act-list-head,
.act-row {
  display: grid;
  grid-template-columns: 1.1fr 1.4fr 1.3fr 0.9fr 0.8fr 0.5fr 1.1fr;
  gap: 10px;
  align-items: center;
  padding: 9px 12px;
  font-size: 13px;
}

.act-list-head {
  background: var(--bg-secondary);
  color: var(--text-secondary);
  font-size: 12px;
}

.act-row + .act-row,
.act-row:first-of-type {
  border-top: 1px solid var(--border-color);
}

.col-name {
  font-weight: 600;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.col-time,
.col-count {
  color: var(--text-secondary);
}

.col-status {
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.col-status.status-off {
  color: var(--text-muted);
  opacity: 0.8;
}

/* 窄窗口:隐藏「状态」列,避免挤压其它列 */
@media (max-width: 680px) {
  .act-list-head,
  .act-row {
    grid-template-columns: 1.1fr 1.3fr 1.2fr 0.75fr 0.5fr 1fr;
  }

  .col-status {
    display: none;
  }
}

/* A63:备注列(截断,全文走 title) */
.col-note {
  color: var(--text-secondary);
  font-style: italic;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.col-proj {
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.act-tag {
  display: inline-block;
  padding: 1px 8px;
  margin: 2px 4px 2px 0;
  border-radius: 999px;
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  font-size: 11px;
  color: var(--text-secondary);
  cursor: pointer;
}

.act-tag.add {
  border-style: dashed;
  color: var(--text-secondary);
  opacity: 0.8;
}

.act-more {
  margin-left: 4px;
  font-size: 11px;
  color: var(--text-secondary);
}

.act-none {
  color: var(--text-secondary);
}

.act-cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
}

.act-card {
  border: 1px solid var(--border-color);
  border-radius: 12px;
  padding: 12px;
  background: var(--bg-secondary);
}

.act-card-top {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 10px;
}

.act-card-name {
  font-weight: 600;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.act-kv {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 12px;
  color: var(--text-secondary);
  padding: 3px 0;
}

.act-kv b {
  color: var(--text-primary);
  font-weight: 500;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
}

.act-kv.proj {
  align-items: flex-start;
}

.act-proj-line {
  max-width: 170px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: right;
}

/* 卡片底部:启用状态 / 余额 / 令牌到期 */
.act-card-foot {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px dashed var(--border-color);
  font-size: 11px;
  color: var(--text-secondary);
}

.act-foot-item {
  white-space: nowrap;
}

.text-danger {
  color: var(--danger);
}

.text-warning {
  color: var(--warning);
}

.act-timeline {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 8px 12px;
}

.act-day {
  margin: 8px 0 4px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.act-event {
  display: grid;
  grid-template-columns: 70px 1.4fr 1.6fr 110px;
  gap: 10px;
  align-items: center;
  padding: 6px 0;
  font-size: 13px;
}

.act-event-wrap {
  border-top: 1px dashed var(--border-color);
}

.act-event-time {
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.act-event-name {
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.act-event-entry {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  text-align: right;
  font-size: 11px;
  color: var(--text-secondary);
}

/* A63:时间线事件备注 */
.act-note-btn {
  border: 1px dashed var(--border-color);
  border-radius: 999px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10px;
  padding: 1px 7px;
  cursor: pointer;
  white-space: nowrap;
}

.act-note-btn:hover {
  color: var(--accent);
  border-color: var(--accent);
}

.act-event-note {
  padding: 0 0 6px 80px;
  font-size: 12px;
  font-style: italic;
  color: var(--text-secondary);
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* A63:卡片视图备注摘要(2 行省略) */
.act-card-note {
  margin: 4px 0 6px;
  font-size: 12px;
  font-style: italic;
  color: var(--text-secondary);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

/* A63:标签胶囊输入 */
.tag-input-box {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  min-height: 38px;
  padding: 6px 8px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-primary, transparent);
  box-sizing: border-box;
}

.tag-input-box .act-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin: 0;
}

.tag-x {
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 12px;
  line-height: 1;
  padding: 0;
}

.tag-x:hover {
  color: var(--danger);
}

.tag-input {
  flex: 1;
  min-width: 120px;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text-primary);
  font-size: 13px;
}

.tag-input::placeholder {
  color: var(--text-muted);
}

.act-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 60;
}

.act-editor {
  width: 360px;
  max-width: 90vw;
  border-radius: 12px;
  padding: 16px;
  background: var(--bg-primary, #fff);
  border: 1px solid var(--border-color);
}

.act-editor h3 {
  margin: 0 0 6px;
  font-size: 15px;
  color: var(--text-primary);
}

.act-editor-hint {
  margin: 0 0 10px;
  font-size: 12px;
  color: var(--text-secondary);
}

.act-editor-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 48px 0;
  color: var(--text-secondary);
  font-size: 13px;
}
/* ===== A61:统计视图 ===== */
.act-stats {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.stat-cards {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
}

.stat-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 14px;
  border: 1px solid var(--border-color);
  border-radius: 12px;
  background: var(--bg-secondary);
}

.stat-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.stat-value {
  font-size: 20px;
  color: var(--text-primary);
}

.stat-section {
  padding: 14px;
  border: 1px solid var(--border-color);
  border-radius: 12px;
}

.stat-section h4 {
  margin: 0 0 8px;
  font-size: 13px;
  color: var(--text-primary);
}

.day-row {
  display: grid;
  grid-template-columns: 52px 1fr 52px 1fr 86px;
  gap: 8px;
  align-items: center;
  padding: 3px 0;
  font-size: 12px;
  color: var(--text-secondary);
}

.day-date {
  font-variant-numeric: tabular-nums;
}

.day-bar-wrap {
  height: 8px;
  border-radius: 999px;
  background: var(--bg-primary, transparent);
  overflow: hidden;
}

.day-bar {
  height: 100%;
  border-radius: 999px;
  min-width: 2px;
}

.day-bar.opens {
  background: var(--accent-color, #3b82f6);
}

.day-bar.time {
  background: var(--success, #10b981);
}

.day-num {
  font-size: 11px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.usage-row {
  display: grid;
  grid-template-columns: 1.6fr 1fr 0.6fr 24px;
  gap: 8px;
  align-items: center;
  padding: 7px 6px;
  border-top: 1px dashed var(--border-color);
  cursor: pointer;
  font-size: 13px;
}

.usage-row:hover {
  background: var(--bg-secondary);
}

.usage-name {
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.usage-time {
  color: var(--text-primary);
  font-weight: 600;
}

.usage-count {
  color: var(--text-secondary);
  font-size: 12px;
}

.usage-caret {
  color: var(--text-secondary);
  text-align: center;
}

.session-list {
  padding: 4px 6px 6px 18px;
}

.session-row {
  display: grid;
  grid-template-columns: 1fr 1fr 0.7fr auto;
  gap: 8px;
  padding: 4px 0;
  font-size: 12px;
  color: var(--text-secondary);
  border-top: 1px dotted var(--border-color);
}

.session-dur {
  color: var(--text-primary);
}

.session-warn {
  color: var(--warning);
}

.usage-more {
  margin-top: 8px;
}

.proj-results {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 8px;
}

.proj-row {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  padding: 6px 0;
  border-top: 1px dashed var(--border-color);
  font-size: 13px;
}

.proj-name {
  min-width: 140px;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.proj-accounts {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.proj-account-link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent-color, #3b82f6);
  cursor: pointer;
  font-size: 12px;
}

.proj-account-link:hover {
  text-decoration: underline;
}

.act-card-highlight {
  border-color: var(--accent-color, #3b82f6) !important;
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.35);
}

.act-stats .act-search {
  width: 100%;
  box-sizing: border-box;
  margin-bottom: 4px;
}
</style>
