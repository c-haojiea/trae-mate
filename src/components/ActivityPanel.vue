<template>
  <div class="activity-page">
    <!-- 工具条 -->
    <div class="act-toolbar">
      <input v-model="search" class="act-search" type="text" placeholder="搜索账号名或标签" />
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

    <!-- 时间线筛选(仅时间线视图) -->
    <div v-if="view === 'timeline'" class="act-filters">
      <div class="act-range">
        <button
          v-for="r in ranges"
          :key="r.key"
          class="act-range-btn"
          :class="{ active: timelineRange === r.key }"
          @click="timelineRange = r.key"
        >
          {{ r.label }}
        </button>
      </div>
      <select v-model="timelineAccount" class="act-select">
        <option value="">全部账号</option>
        <option v-for="a in timelineAccountOptions" :key="a.id" :value="a.id">
          {{ a.name }}
        </option>
      </select>
    </div>

    <p class="act-hint">标签为手动维护，用于标记该账号的用途；项目路径自动提取在 SOLO 模式下常为空。历史记录显示的是打开当时的标签。</p>

    <!-- 无打开记录提示(仍保留账号列表以便维护标签) -->
    <div v-if="!loading && store.activityEvents.length === 0" class="act-empty-hint">
      还没有打开记录，从账号卡片启动一次即可产生
    </div>

    <!-- 空态 -->
    <div v-if="!loading && filtered.length === 0" class="empty-state">
      <Icon name="inbox" :size="40" />
      <p>没有匹配的账号</p>
    </div>

    <!-- 列表视图 -->
    <div v-else-if="view === 'list'" class="act-list">
      <div class="act-list-head">
        <span class="col-name">账号</span>
        <span class="col-tags">标签</span>
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
      <div v-for="a in filtered" :key="a.id" class="act-card">
        <div class="act-card-top">
          <span class="act-card-name">{{ a.name }}</span>
          <span class="act-tags" @click="openEditor(a)">
            <template v-if="a.tags.length">
              <span v-for="t in a.tags" :key="t" class="act-tag">{{ t }}</span>
            </template>
            <span v-else class="act-tag add">+ 标签</span>
          </span>
        </div>
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

    <!-- 时间线视图 -->
    <div v-else class="act-timeline">
      <div v-if="filteredTimeline.length === 0" class="act-empty-hint">该范围内没有打开记录</div>
      <template v-for="g in timelineGroups" :key="g.date">
        <div class="act-day">{{ g.label }}</div>
        <div v-for="(e, i) in g.items" :key="g.date + '-' + i" class="act-event">
          <span class="act-event-time">{{ fmtTime(e.at) }}</span>
          <span class="act-event-name">{{ e.name }}</span>
          <span class="act-tags">
            <span v-for="t in e.tags" :key="t" class="act-tag">{{ t }}</span>
          </span>
          <span class="act-event-entry">{{ entryLabel(e.entry) }}</span>
        </div>
      </template>
    </div>

    <!-- 标签编辑弹窗 -->
    <div v-if="editing" class="act-mask" @click.self="closeEditor">
      <div class="act-editor">
        <h3>{{ editing.name }} · 标签</h3>
        <p class="act-editor-hint">用逗号或回车分隔，最多 8 个，单个不超过 24 字</p>
        <input
          ref="editorInput"
          v-model="editorText"
          class="act-search"
          type="text"
          placeholder="例如：Anki分析, 视频下载器"
          @keydown.enter.prevent="saveTags"
        />
        <div class="act-editor-actions">
          <button class="btn btn-sm btn-outline" @click="closeEditor">取消</button>
          <button class="btn btn-sm btn-primary" :disabled="saving" @click="saveTags">保存</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onActivated, onMounted, ref, watch } from 'vue'
import { ACTIVITY_VIEW_KEY, useAppStore, type ActivityAccount } from '../stores/app'
import Icon from './Icon.vue'

const emit = defineEmits(['notify'])
const store = useAppStore()

const loading = ref(false)
const search = ref('')
const sortBy = ref<'recent' | 'name' | 'count'>('recent')
const onlyTagged = ref(false)

const activityViews = ['list', 'card', 'timeline'] as const
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
]

async function reload() {
  loading.value = true
  try {
    await store.fetchAccountActivity()
  } finally {
    loading.value = false
  }
}

onMounted(reload)
// 被 KeepAlive 缓存后,每次切回该页签都要重新拉一次(否则启动实例后仍显示「从未」)
onActivated(reload)

// 三视图共用同一份筛选结果(切换视图不重新请求)
const filtered = computed(() => {
  const kw = search.value.trim().toLowerCase()
  let list = store.activityAccounts.filter((a) => {
    if (onlyTagged.value && a.tags.length === 0) return false
    if (!kw) return true
    return a.name.toLowerCase().includes(kw) || a.tags.some((t) => t.toLowerCase().includes(kw))
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

// ===== 时间线筛选 =====
const ranges = [
  { key: 'today', label: '今天' },
  { key: '7d', label: '近7天' },
  { key: '30d', label: '近30天' },
  { key: 'all', label: '全部' },
] as const
const timelineRange = ref<'today' | '7d' | '30d' | 'all'>('today')
const timelineAccount = ref('')

// 账号下拉:仅列出有事件的账号,按最近打开倒序
const timelineAccountOptions = computed(() =>
  store.activityAccounts
    .filter((a) => store.activityEvents.some((e) => e.accountId === a.id))
    .slice()
    .sort((a, b) => (b.lastOpenedAt || 0) - (a.lastOpenedAt || 0))
)

// Asia/Shanghai 固定 UTC+8(无夏令时),返回该毫秒时刻所属自然日的 0 点
function startOfShanghaiDay(ms: number) {
  const shifted = new Date(ms + 8 * 3_600_000)
  const dayStartUtc = Date.UTC(
    shifted.getUTCFullYear(),
    shifted.getUTCMonth(),
    shifted.getUTCDate()
  )
  return dayStartUtc - 8 * 3_600_000
}

const filteredTimeline = computed(() => {
  const now = Date.now()
  const cutoff =
    timelineRange.value === 'today'
      ? startOfShanghaiDay(now)
      : timelineRange.value === '7d'
        ? now - 7 * 86_400_000
        : timelineRange.value === '30d'
          ? now - 30 * 86_400_000
          : 0
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

// ===== 标签编辑 =====
const editing = ref<ActivityAccount | null>(null)
const editorText = ref('')
const saving = ref(false)
const editorInput = ref<HTMLInputElement | null>(null)

function openEditor(a: ActivityAccount) {
  editing.value = { ...a }
  editorText.value = a.tags.join(', ')
  nextTick(() => editorInput.value?.focus())
}

function closeEditor() {
  editing.value = null
}

async function saveTags() {
  if (!editing.value || saving.value) return
  const tags = editorText.value
    .split(/[,，\n]/)
    .map((s) => s.trim())
    .filter(Boolean)
  saving.value = true
  try {
    await store.updateAccountTags(editing.value.id, tags)
    emit('notify', '标签已保存', 'success')
    editing.value = null
  } catch (e: any) {
    emit('notify', String(e?.message || e || '保存标签失败'), 'error')
  } finally {
    saving.value = false
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
  grid-template-columns: 1.2fr 1.5fr 1fr 0.85fr 0.55fr 1.2fr;
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
    grid-template-columns: 1.2fr 1.4fr 0.9fr 0.55fr 1.2fr;
  }

  .col-status {
    display: none;
  }
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
  grid-template-columns: 70px 1.4fr 1.6fr 70px;
  gap: 10px;
  align-items: center;
  padding: 6px 0;
  font-size: 13px;
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
  text-align: right;
  font-size: 11px;
  color: var(--text-secondary);
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
</style>
