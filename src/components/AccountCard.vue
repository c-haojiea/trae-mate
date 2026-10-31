<template>
  <div class="account-card card" :class="{ disabled: !account.enabled, 'instance-dead': instanceDead }">
    <!-- 卡片头部 -->
    <div class="card-header">
      <div class="account-info">
        <div class="avatar">{{ account.name.charAt(0).toUpperCase() }}</div>
        <div class="account-name-wrap">
          <h3 class="account-name">
            {{ account.name }}
            <span v-if="instanceState.isMainAccount" class="badge-main">主账号</span>
            <span v-if="instanceDead" class="badge-dead">实例未登录</span>
          </h3>
          <span class="badge" :class="statusBadgeClass">
            {{ statusText }}
          </span>
        </div>
      </div>
      <div class="switch" :class="{ active: account.enabled }" @click="handleToggle">
      </div>
    </div>

    <!-- 卡片内容 -->
    <div class="card-body">
      <div class="info-row">
        <span class="info-label">上次签到</span>
        <span class="info-value">{{ lastCheckinText }}</span>
      </div>
      <div class="info-row">
        <span class="info-label">总积分</span>
        <span class="info-value points">
          {{ account.points || 0 }} 分
        </span>
      </div>
      <div class="info-row">
        <span class="info-label">录入</span>
        <span class="info-value">{{ formatDate(account.createdAt) }}（第 {{ entryDay }} 天）</span>
      </div>
      <div class="info-row" v-if="account.source !== 'jwt' && account.credentialStatus">
        <span class="info-label">桌面凭证</span>
        <span class="cred-value-wrap">
          <span class="info-value" :class="account.credentialStatus === 'expired' ? 'text-danger' : 'text-success'">
            {{ credentialStatusText }}
          </span>
          <button
            class="btn-refresh-cred"
            title="回读 TRAE 最新凭证(打开 TRAE 让它自行刷新后,点此同步)"
            @click="handleRefreshCred"
            :disabled="refreshingCred"
          >
            <Icon name="arrow-path" :size="11" :class="{ spinning: refreshingCred }" />
          </button>
        </span>
      </div>
      <div class="info-row" v-if="account.expiresAt || instanceDead">
        <span class="info-label">令牌到期</span>
        <span class="info-value" :class="tokenExpireClass">{{ tokenExpireText }}</span>
      </div>
      <div class="info-row" v-if="isJwt">
        <span class="info-label">JWT 状态</span>
        <span class="cred-value-wrap">
          <span class="info-value" :class="jwtStatusClass">{{ jwtStatusText }}</span>
          <button
            v-if="account.hasRefreshToken"
            class="btn-refresh-cred"
            title="用 refresh_token 自动刷新 JWT"
            @click="handleRefreshJwt"
            :disabled="refreshingJwt"
          >
            <Icon name="arrow-path" :size="11" :class="{ spinning: refreshingJwt }" />
          </button>
        </span>
      </div>
      <div class="info-row" v-if="account.cooldownType">
        <span class="info-label">冷却中</span>
        <span class="cred-value-wrap">
          <span class="info-value text-danger">{{ cooldownText }}</span>
          <button class="btn-refresh-cred" title="清除冷却状态" @click="handleClearCooldown">
            <Icon name="x-mark" :size="11" />
          </button>
        </span>
      </div>
    </div>

    <!-- 卡片底部 -->
    <div class="card-footer">
      <button
        class="btn btn-sm btn-primary"
        @click="handleCheckin"
        :disabled="checkingIn || store.checkingIn || !account.enabled"
      >
        <span v-if="checkingIn" class="spinner"></span>
        <span>{{ checkingIn ? '签到中' : '立即签到' }}</span>
      </button>
      <template v-if="!isJwt">
        <button
          v-if="instanceState.source === 'none'"
          class="btn btn-sm btn-outline"
          @click="handleLaunchMulti"
          :disabled="launching"
          :title="account.dataDir ? '启动该账号的独立实例' : '首次启动:创建独立实例并启动'"
        >
          <span v-if="launching" class="spinner"></span>
          <span>{{ launching ? '程序正在启动,请稍后' : '启动' }}</span>
        </button>
        <button
          v-else
          class="btn btn-sm btn-outline"
          @click="handleFocus"
          :title="instanceState.source === 'main' ? '该账号在主实例运行,点击聚焦主实例窗口' : '该账号工具实例已运行,点击聚焦其窗口'"
        >
          <span>聚焦</span>
        </button>
      </template>
      <button type="button" class="btn btn-sm btn-outline" @click.stop="startEdit">
        编辑
      </button>
      <button class="btn btn-sm btn-outline danger" @click="handleDelete">
        删除
      </button>
    </div>
  </div>
  <div v-if="editing" class="edit-overlay" @click.self="cancelEdit">
    <div class="edit-dialog">
      <h3>编辑账号名称</h3>
      <input v-model="draftName" class="input" autofocus @keyup.enter="saveEdit" @keyup.esc="cancelEdit" />
      <div class="edit-actions"><button class="btn btn-sm btn-outline" @click="cancelEdit">取消</button><button class="btn btn-sm btn-primary" @click="saveEdit">保存</button></div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useAppStore, type Account, type InstanceState } from '../stores/app'

const props = defineProps<{
  account: Account
}>()

const emit = defineEmits(['toggle', 'delete', 'edit', 'notify'])
const store = useAppStore()

const checkingIn = ref(false)
const launching = ref(false)
const refreshingCred = ref(false)
const refreshingJwt = ref(false)
const instanceState = ref<InstanceState>({ running: false, source: 'none', isMainAccount: false })

const isJwt = computed(() => props.account.source === 'jwt')

onMounted(async () => {
  instanceState.value = await store.getInstanceState(props.account.id)
  startInstancePoll()
})

// 定期轮询实例状态:用户可能在本应用之外关闭 TRAE 窗口,需及时把「聚焦」还原成「启动」
let pollTimer: number | undefined
function startInstancePoll() {
  stopInstancePoll()
  pollTimer = window.setInterval(async () => {
    if (instanceState.value.source === 'none') return // 未运行时不轮询,避免无谓开销
    const s = await store.getInstanceState(props.account.id)
    instanceState.value = s
    if (s.source === 'none') stopInstancePoll()
  }, 5000)
}
function stopInstancePoll() {
  if (pollTimer !== undefined) {
    window.clearInterval(pollTimer)
    pollTimer = undefined
  }
}
onUnmounted(stopInstancePoll)

// 监听全局刷新信号(顶部"刷新"按钮触发),重新查询本账号实例运行状态(含主/工具来源)
watch(() => store.instanceRefreshTick, async () => {
  instanceState.value = await store.getInstanceState(props.account.id)
})
const editing = ref(false)
const draftName = ref(props.account.name)

const statusText = computed(() => {
  if (!props.account.enabled) return '已停用'
  if (!props.account.lastCheckinAt) return '未签到'

  const today = new Date().toDateString()
  const lastDate = new Date(props.account.lastCheckinAt).toDateString()

  if (lastDate === today) {
    if (props.account.lastCheckinResult === 'success') {
      return instanceDead.value ? '今日已签（已失效）' : '今日已签'
    }
    return '今日失败'
  }
  return '待签到'
})

const statusBadgeClass = computed(() => {
  if (!props.account.enabled) return 'badge-muted'
  if (!props.account.lastCheckinAt) return 'badge-warning'

  const today = new Date().toDateString()
  const lastDate = new Date(props.account.lastCheckinAt).toDateString()

  if (lastDate === today) {
    if (props.account.lastCheckinResult === 'success') {
      return instanceDead.value ? 'badge-muted' : 'badge-success'
    }
    return 'badge-danger'
  }
  return 'badge-info'
})

const lastCheckinText = computed(() => {
  if (!props.account.lastCheckinAt) return '从未签到'
  return formatDateTime(props.account.lastCheckinAt)
})

const credentialStatusText = computed(() => {
  if (props.account.credentialStatus === 'expired') return '凭证已失效，请打开 TRAE 实例刷新'
  if (props.account.credentialStatus === 'expiring') return '凭证即将过期，打开 TRAE 实例续期'
  return '凭证有效'
})

const jwtStatusClass = computed(() => {
  switch (props.account.jwtStatus) {
    case 'ok': return 'text-success'
    case 'warn': return 'text-warning'
    case 'expired': return 'text-danger'
    default: return ''
  }
})

const jwtStatusText = computed(() => {
  const status = props.account.jwtStatus
  if (status === 'expired') return 'JWT 已过期'
  if (status === 'unknown') return 'JWT 未知'
  const h = props.account.jwtExpTimestamp
    ? (props.account.jwtExpTimestamp - Date.now() / 1000) / 3600
    : null
  if (status === 'warn' && h != null) return `JWT ${h.toFixed(1)}h 后过期`
  return 'JWT 有效'
})

const cooldownText = computed(() => {
  const t = props.account.cooldownType
  const until = props.account.cooldownUntil || 0
  const reason = props.account.cooldownReason
  if (until >= 9999999999) return `${t}：永久冷却（需重新登录/刷新凭证）`
  const mins = Math.ceil((until - Date.now() / 1000) / 60)
  return `${t}${reason ? '：' + reason : ''} · 剩余 ${Math.max(mins, 0)} 分钟`
})

function handleToggle() {
  emit('toggle', props.account.id, !props.account.enabled)
}

// 签到由卡片自身执行并等待完成,按钮在整个请求期间保持禁用(防止连点触发服务端限频)
async function handleCheckin() {
  if (checkingIn.value) return
  checkingIn.value = true
  try {
    const result = await store.checkinAccount(props.account.id)
    let msg = result?.message || (result?.success ? '签到成功' : '签到失败')
    if (msg.includes('频繁')) msg += ',请稍候一分钟再试'
    emit('notify', msg, result?.success ? 'success' : 'error')
    if (result?.success) {
      const points = await store.getAccountPoints(props.account.id)
      emit('notify',
        points?.success ? '签到后总积分已更新' : (points?.message || '签到成功，但总积分查询失败'),
        points?.success ? 'success' : 'error')
    }
  } catch (e: any) {
    emit('notify', '签到失败: ' + (e?.message || e), 'error')
  } finally {
    checkingIn.value = false
  }
}

async function handleLaunchMulti() {
  launching.value = true
  try {
    await store.launchMulti(props.account.id)
    instanceState.value = { running: true, source: 'tool', isMainAccount: instanceState.value.isMainAccount }
    startInstancePoll()
    emit('notify', '实例已启动', 'success')
  } catch (e: any) {
    emit('notify', '启动失败: ' + (e?.message || e), 'error')
    // 失败可能因状态过时(如该账号已在主实例运行),重新查询修正显示
    instanceState.value = await store.getInstanceState(props.account.id)
  } finally {
    launching.value = false
  }
}

async function handleFocus() {
  try {
    await store.focusInstance(props.account.id)
  } catch (e: any) {
    // 实例可能已被用户在外部关闭:回查状态,让按钮自动还原为「启动」
    const s = await store.getInstanceState(props.account.id)
    instanceState.value = s
    if (s.source === 'none') {
      stopInstancePoll()
      emit('notify', '该账号实例已关闭,按钮已还原为「启动」', 'success')
    } else {
      emit('notify', '聚焦失败: ' + (e?.message || e), 'error')
    }
  }
}

function handleDelete() {
  emit('delete', props.account.id)
}

// 回读最新凭证:打开 TRAE 实例让它自行刷新 token,点此同步到应用(应用不主动刷新 token)
async function handleRefreshCred() {
  if (refreshingCred.value) return
  refreshingCred.value = true
  try {
    await store.refreshCredential(props.account.id)
    emit('notify', '已回读 TRAE 最新凭证', 'success')
  } catch (e: any) {
    emit('notify', '回读凭证失败: ' + (e?.message || e), 'error')
  } finally {
    refreshingCred.value = false
  }
}

// 刷新 JWT 账号令牌(refresh_token -> ExchangeToken)
async function handleRefreshJwt() {
  if (refreshingJwt.value || !props.account.userId) return
  refreshingJwt.value = true
  try {
    await store.refreshJwt(props.account.userId)
    emit('notify', 'JWT 已刷新', 'success')
  } catch (e: any) {
    emit('notify', 'JWT 刷新失败: ' + (e?.message || e), 'error')
  } finally {
    refreshingJwt.value = false
  }
}

// 清除冷却状态
async function handleClearCooldown() {
  if (!props.account.userId) return
  try {
    await store.clearCooldown(props.account.userId)
    emit('notify', '冷却已清除', 'success')
  } catch (e: any) {
    emit('notify', '清除冷却失败: ' + (e?.message || e), 'error')
  }
}

function startEdit() { draftName.value = props.account.name; editing.value = true }
function cancelEdit() { editing.value = false }
function saveEdit() { const name = draftName.value.trim(); if (name && name !== props.account.name) emit('edit', props.account.id, name); editing.value = false }


function formatDate(timestamp: number): string {
  const date = new Date(timestamp)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

function formatDateTimeFull(timestamp: number): string {
  const date = new Date(timestamp)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')} ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}

// 实例是否仍登录:仅 false 才标记,null/undefined 不动(避免误伤未绑定目录的账号)
const instanceDead = computed(() => props.account.instanceLoggedIn === false)

// 录入第 N 天:录入当天算第 1 天
const entryDay = computed(() => Math.floor((Date.now() - props.account.createdAt) / 86400000) + 1)

// 令牌到期:>7 天正常色、3-7 天橙色、<3 天或已过期红色
const tokenExpireText = computed(() => {
  // 实例未登录 = 该令牌已无法续期,直接提示重扫
  if (instanceDead.value) return '令牌已失效，需重扫'
  const exp = props.account.expiresAt
  if (!exp) return ''
  const left = exp - Date.now()
  if (left <= 0) return `${formatDateTimeFull(exp)} 已过期`
  const days = Math.floor(left / 86400000)
  return `${formatDateTimeFull(exp)} 剩余 ${days} 天`
})

const tokenExpireClass = computed(() => {
  if (instanceDead.value) return 'text-danger'
  const exp = props.account.expiresAt
  if (!exp) return ''
  const left = exp - Date.now()
  const days = Math.floor(left / 86400000)
  if (left <= 0 || days < 3) return 'text-danger'
  if (days < 7) return 'text-warning'
  return ''
})

function formatDateTime(timestamp: number): string {
  const date = new Date(timestamp)
  const now = new Date()
  const diff = now.getTime() - date.getTime()

  if (diff < 60000) return '刚刚'
  if (diff < 3600000) return `${Math.floor(diff / 60000)} 分钟前`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)} 小时前`

  return `${date.getMonth() + 1}/${date.getDate()} ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}
</script>

<style scoped>
/* 账号卡片 - 玻璃,hover 上浮 */
.account-card {
  padding: 20px;
  transition: box-shadow var(--t-smooth), transform var(--t-smooth), border-color var(--t-smooth);
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-soft-raised);
}

.account-card:hover {
  box-shadow: var(--shadow-md-raised);
  border-color: var(--glass-border-strong);
  transform: translateY(-3px);
}

.account-card.disabled {
  opacity: 0.55;
}

/* 实例未登录:整体置灰 + 左侧红条 */
.account-card.instance-dead {
  opacity: 0.62;
  border-left: 3px solid var(--danger);
}

.badge-dead {
  display: inline-block;
  margin-left: 6px;
  padding: 1px 6px;
  border-radius: var(--r-sm);
  background: rgba(220, 38, 38, 0.16);
  color: var(--danger);
  font-size: 10px;
  font-weight: 700;
  vertical-align: middle;
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 16px;
}

.account-info {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  min-width: 0;
}

/* 头像 - 渐变强调色 + 白字(品牌标识,小面积) */
.avatar {
  width: 44px;
  height: 44px;
  border-radius: 14px;
  background: var(--accent-grad);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  font-weight: 700;
  font-family: 'Nunito', 'Inter', sans-serif;
  box-shadow: 0 6px 16px rgba(0, 180, 216, 0.30), inset 0 1px 0 rgba(255, 255, 255, 0.35);
  flex-shrink: 0;
}

.account-name-wrap {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.account-name {
  font-size: 15px;
  font-weight: 700;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.badge-main {
  display: inline-block;
  margin-left: 8px;
  padding: 2px 8px;
  font-size: 11px;
  font-weight: 600;
  color: #fff;
  background: var(--accent-grad);
  border-radius: 6px;
  vertical-align: middle;
  box-shadow: 0 2px 8px rgba(0, 180, 216, 0.28);
}

.badge {
  width: fit-content;
  align-self: flex-start;
  white-space: nowrap;
}

/* 卡片内容区 - 纯靠间距与底部按钮区分隔,不用边框线(Neumorphism 无 border) */
.card-body {
  margin-bottom: 18px;
}

.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 0;
}

.info-label {
  font-size: 13px;
  color: var(--text-secondary);
}

.info-value {
  font-size: 13px;
  color: var(--text-primary);
  font-weight: 600;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.info-value.points {
  color: var(--warning);
  font-weight: 700;
}

/* 桌面凭证行:状态 + 手动刷新小按钮 */
.cred-value-wrap {
  display: flex;
  align-items: center;
  gap: 6px;
}

.btn-refresh-cred {
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 50%;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface);
  color: var(--text-muted);
  box-shadow: var(--shadow-soft-raised);
  transition: box-shadow var(--t-press), color var(--t-smooth);
  flex-shrink: 0;
}

.btn-refresh-cred:hover:not(:disabled) {
  color: var(--accent);
}

.btn-refresh-cred:active:not(:disabled) {
  box-shadow: var(--shadow-soft-inset);
}

.btn-refresh-cred:disabled {
  cursor: default;
  color: var(--accent);
}

.spinning {
  animation: cred-spin 0.8s linear infinite;
}

@keyframes cred-spin {
  to {
    transform: rotate(360deg);
  }
}

.text-success {
  color: var(--success);
}

.text-warning {
  color: var(--warning);
}

.text-danger {
  color: var(--danger);
}

.card-footer {
  display: flex;
  gap: 8px;
}

.card-footer .btn {
  flex: 1;
}

/* 危险轮廓按钮 - neu 版 */
.btn-outline.danger {
  color: var(--danger);
}

.btn-outline.danger:hover:not(:disabled) {
  color: var(--danger);
  box-shadow: var(--shadow-soft-raised);
}

.btn-outline.danger:active:not(:disabled) {
  box-shadow: var(--shadow-soft-inset);
}

/* 编辑弹窗 */
.edit-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(15, 23, 42, 0.35);
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
}

.edit-dialog {
  width: 360px;
  padding: 22px;
  border-radius: var(--r-lg);
  background: var(--glass-bg-strong);
  border: 1px solid var(--glass-border);
  box-shadow: var(--shadow-bold-raised);
}

.edit-dialog h3 {
  font-size: 16px;
  font-weight: 700;
  color: var(--text-primary);
}

.edit-dialog .input {
  width: 100%;
  box-sizing: border-box;
  margin: 16px 0;
}

.edit-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
