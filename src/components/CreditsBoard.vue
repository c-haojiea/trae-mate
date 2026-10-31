<template>
  <section class="credits-board card">
    <div
      class="cb-header"
      role="button"
      tabindex="0"
      @click="expanded = !expanded"
      @keydown.enter="expanded = !expanded"
    >
      <span class="cb-caret">{{ expanded ? '▾' : '▸' }}</span>
      <h2 class="cb-title">积分看板</h2>
      <span class="cb-summary">
        {{ todaySnap ? `今日消耗 ${fmt(todaySnap.consumed)} · 新增 ${fmt(todaySnap.earned)}` : '今日尚未刷新积分' }}
      </span>
    </div>

    <div v-if="expanded" class="cb-body">
      <!-- 今日三项 -->
      <div class="cb-today">
        <div class="cb-stat">
          <span class="cb-stat-label">今日消耗</span>
          <span class="cb-stat-value">{{ todaySnap ? fmt(todaySnap.consumed) : '—' }}</span>
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
      <p v-if="!todaySnap" class="cb-hint">今日尚未刷新积分</p>

      <!-- 近 7 天 -->
      <div class="cb-section">
        <h3 class="cb-section-title">近 7 天</h3>
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
              <td class="num">{{ fmt(s.consumed) }}</td>
              <td class="num">{{ fmt(s.total) }}</td>
            </tr>
            <tr v-if="recent.length === 0">
              <td colspan="4" class="cb-empty">暂无快照</td>
            </tr>
          </tbody>
        </table>
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

const store = useAppStore()
const expanded = ref(false)

// 北京时间今天,格式 YYYY-MM-DD(与后端 credits_daily.json 的 date 对齐)
const todayStr = computed(() =>
  new Date().toLocaleDateString('en-CA', { timeZone: 'Asia/Shanghai' })
)

const todaySnap = computed(() =>
  store.creditsDaily.find((s: { date: string }) => s.date === todayStr.value)
)

// 近 7 天,按日期倒序
const recent = computed(() =>
  [...store.creditsDaily].sort((a, b) => b.date.localeCompare(a.date)).slice(0, 7)
)

// 每账号当前余额,降序(未知余额排最后)
const balances = computed(() =>
  [...store.accounts].sort((a, b) => (b.remainingCredits ?? -1) - (a.remainingCredits ?? -1))
)

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

.cb-body {
  padding: 0 18px 16px;
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

.cb-section {
  margin-top: 14px;
}

.cb-section-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-secondary);
  margin-bottom: 8px;
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

.cb-empty {
  font-size: 12px;
  color: var(--text-muted);
  padding: 8px 6px;
  text-align: center;
}
</style>
