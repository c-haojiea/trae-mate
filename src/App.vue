<template>
  <div class="app-container">
    <!-- 背景极光层(纯装饰) -->
    <div class="aurora" aria-hidden="true">
      <span class="aurora-blob blob-a"></span>
      <span class="aurora-blob blob-b"></span>
      <span class="aurora-blob blob-c"></span>
    </div>

    <div class="app-body">
    <!-- 侧边栏 -->
    <aside class="sidebar">
      <!-- 窗口操作栏 - 窗口控件左、主题切换右 -->
      <div class="window-bar" data-tauri-drag-region="deep">
        <div class="window-controls">
          <button class="win-btn" title="最小化" @click="minimizeWindow"><Icon name="minus" :size="13" /></button>
          <button class="win-btn" :title="isMaximized ? '向下还原' : '最大化'" @click="toggleMaximize">
            <Icon :name="isMaximized ? 'restore' : 'maximize'" :size="13" />
          </button>
          <button class="win-btn close" title="关闭" @click="closeWindow"><Icon name="x-mark" :size="13" /></button>
        </div>
        <button class="win-btn theme-toggle" :title="isDark ? '切换到亮色模式' : '切换到暗色模式'" @click="toggleTheme">
          <Icon :name="isDark ? 'sun' : 'moon'" :size="16" />
        </button>
      </div>

      <div class="sidebar-header">
        <div class="logo">
          <span class="logo-icon"><Icon name="check" :size="20" /></span>
          <span class="logo-text">TraeMate</span>
        </div>
        <p class="logo-desc">签到与多开账号管理</p>
      </div>

      <nav class="nav-menu">
        <div
          class="nav-item"
          role="button"
          tabindex="0"
          :class="{ active: activeTab === 'accounts' }"
          @click="activeTab = 'accounts'"
          @keydown.enter="activeTab = 'accounts'"
        >
          <span class="nav-icon"><Icon name="users" :size="18" /></span>
          <span>账号管理</span>
          <span class="nav-badge" v-if="store.accounts.length">{{ store.accounts.length }}</span>
        </div>
        <div
          class="nav-item"
          role="button"
          tabindex="0"
          :class="{ active: activeTab === 'logs' }"
          @click="activeTab = 'logs'"
          @keydown.enter="activeTab = 'logs'"
        >
          <span class="nav-icon"><Icon name="clipboard-list" :size="18" /></span>
          <span>签到日志</span>
        </div>
        <div
          class="nav-item"
          role="button"
          tabindex="0"
          :class="{ active: activeTab === 'settings' }"
          @click="activeTab = 'settings'"
          @keydown.enter="activeTab = 'settings'"
        >
          <span class="nav-icon"><Icon name="cog" :size="18" /></span>
          <span>设置</span>
        </div>
      </nav>

      <div class="sidebar-footer">
        <div class="status-card">
          <div class="status-item">
            <span class="status-label">自动签到</span>
            <span class="status-value" :class="store.settings?.autoCheckin ? 'on' : 'off'">
              {{ store.settings?.autoCheckin ? '已开启' : '已关闭' }}
            </span>
          </div>
          <div class="status-item" v-if="store.settings?.autoCheckin">
            <span class="status-label">签到时间</span>
            <span class="status-value">{{ store.settings?.checkinTime }}</span>
          </div>
          <div class="status-item">
            <span class="status-label">今日已签</span>
            <span class="status-value success">{{ store.todayCheckedCount }} / {{ store.enabledAccounts.length }}</span>
          </div>
        </div>
      </div>
    </aside>

    <!-- 主内容区 -->
    <div v-if="toastMessage" class="top-toast" :class="toastType">{{ toastMessage }}</div>

    <main class="main-content">
      <!-- 顶部栏 -->
      <header class="top-bar" data-tauri-drag-region="deep">
        <div class="top-bar-left">
          <h1 class="page-title">{{ currentPageTitle }}</h1>
          <p class="page-subtitle" v-if="activeTab === 'accounts'">管理你的 Trae Work 账号，一键签到</p>
          <p class="page-subtitle" v-else-if="activeTab === 'logs'">查看所有签到记录和结果</p>
          <p class="page-subtitle" v-else>配置自动签到和通知设置</p>
        </div>
        <div class="top-bar-right">
          <button
            class="btn btn-outline"
            @click="handleRefresh"
            :disabled="refreshing"
            v-if="activeTab === 'accounts'"
            title="刷新账号与实例状态"
          >
            <Icon name="arrow-path" :size="16" :class="{ spinning: refreshing }" />
            <span>刷新</span>
          </button>
          <button
            class="btn btn-primary"
            @click="handleCheckinAll"
            :disabled="store.checkingIn || store.enabledAccounts.length === 0"
            v-if="activeTab === 'accounts'"
          >
            <span v-if="store.checkingIn" class="spinner"></span>
            <span v-if="store.checkingIn && store.checkinTotal > 0">
              签到中... {{ store.progressDone }}/{{ store.checkinTotal }}
            </span>
            <span v-else>{{ store.checkingIn ? '签到中...' : '一键签到全部' }}</span>
          </button>
          <button
            class="btn btn-primary"
            @click="showAddModal = true"
            v-if="activeTab === 'accounts'"
          >
            <Icon name="plus" :size="16" />
            <span>添加账号</span>
          </button>
        </div>
      </header>

      <div v-if="store.waitingLogin" class="login-waiting-banner">
        正在等待新实例登录,登录后将自动导入账号…
      </div>

      <!-- 积分看板(默认折叠,点标题展开) -->
      <CreditsBoard />

      <!-- 内容区域 -->
      <div class="content-area">
        <AccountList v-if="activeTab === 'accounts'" @add-account="showAddModal = true" @notify="showToast" />
        <CheckinLog v-else-if="activeTab === 'logs'" />
        <SettingsPanel v-else @notify="showToast" />
      </div>
    </main>
    </div><!-- /app-body -->

    <!-- 添加账号弹窗 -->
    <AddAccountModal
      v-model:visible="showAddModal"
      @success="handleAddSuccess"
      @notify="showToast"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { useAppStore } from './stores/app'
import AccountList from './components/AccountList.vue'
import CheckinLog from './components/CheckinLog.vue'
import SettingsPanel from './components/SettingsPanel.vue'
import AddAccountModal from './components/AddAccountModal.vue'
import CreditsBoard from './components/CreditsBoard.vue'
import Icon from './components/Icon.vue'

const appWindow = getCurrentWindow()

const store = useAppStore()
const activeTab = ref<'accounts' | 'logs' | 'settings'>('accounts')
const showAddModal = ref(false)
const toastMessage = ref('')
const toastType = ref<'success' | 'error'>('success')
let toastTimer: ReturnType<typeof setTimeout> | undefined

function showToast(message: string, type: 'success' | 'error' = 'success') {
  toastMessage.value = message
  toastType.value = type
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => { toastMessage.value = '' }, 3500)
}

function closeWindow() {
  appWindow.close()
}

function minimizeWindow() {
  appWindow.minimize()
}

// 最大化 / 还原(状态与窗口实际状态保持同步,含系统快捷键、双击标题栏触发的变更)
const isMaximized = ref(false)

async function syncMaximized() {
  try {
    isMaximized.value = await appWindow.isMaximized()
  } catch {
    /* 忽略:极少数窗口未就绪的情况 */
  }
}

async function toggleMaximize() {
  try {
    await appWindow.toggleMaximize()
    await syncMaximized()
  } catch (e) {
    console.error('切换最大化失败:', e)
  }
}

// 双击拖拽区最大化/还原由 Tauri 原生 drag-region 处理(data-tauri-drag-region),
// 不要再绑 @dblclick,否则会与原生 toggle 叠加成"最大化后立即还原"。

// 明暗主题切换(localStorage 持久化,main.ts 已完成初始设置)
const isDark = ref(document.documentElement.dataset.theme === 'dark')

function toggleTheme() {
  isDark.value = !isDark.value
  const theme = isDark.value ? 'dark' : 'light'
  document.documentElement.dataset.theme = theme
  localStorage.setItem('traemate-theme', theme)
}

const currentPageTitle = computed(() => {
  switch (activeTab.value) {
    case 'accounts': return '账号管理'
    case 'logs': return '签到日志'
    case 'settings': return '设置'
    default: return ''
  }
})

async function handleCheckinAll() {
  if (store.enabledAccounts.length === 0) {
    alert('请先添加并启用账号')
    return
  }
  await store.checkinAll()
}

function handleAddSuccess() {
  showAddModal.value = false
}

const refreshing = ref(false)

// 刷新账号数据 + 触发各卡片重查多开实例运行状态(关闭实例后状态不自动更新,手动刷新)
async function handleRefresh() {
  if (refreshing.value) return
  refreshing.value = true
  try {
    void store.fetchAccounts()
    void store.fetchCreditsDaily()
    store.refreshInstances()
  } catch (e) {
    console.error('刷新失败:', e)
  } finally {
    setTimeout(() => { refreshing.value = false }, 600)
  }
}

onMounted(() => {
  store.init()
  void syncMaximized()
  void appWindow.onResized(() => { void syncMaximized() })
  // 监听系统托盘"一键签到"菜单
  listen('tray-checkin', () => {
    handleCheckinAll()
  })
  // 监听"打开新实例登录"导入结果(后端轮询登录完成后 emit)
  listen('login-imported', (event) => {
    const p = event.payload as { success: boolean; name?: string; error?: string }
    store.waitingLogin = false
    if (p.success) {
      showToast(`账号「${p.name || ''}」已自动导入`, 'success')
      store.fetchAccounts()
    } else {
      showToast(`导入失败: ${p.error || '未知错误'}`, 'error')
    }
  })
})
</script>

<style scoped>
.app-container {
  position: relative;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100vh;
  overflow: hidden;
  background: transparent;
}

/* 背景极光 - 极缓慢漂移,低饱和度,仅为玻璃提供层次 */
.aurora {
  position: fixed;
  inset: -12%;
  z-index: 0;
  pointer-events: none;
  overflow: hidden;
}

.aurora-blob {
  position: absolute;
  border-radius: 50%;
  opacity: 0.85;
}

.blob-a {
  width: 52vw;
  height: 52vw;
  left: -10vw;
  top: -18vw;
  background: radial-gradient(closest-side, var(--aurora-1), transparent 72%);
}

.blob-b {
  width: 46vw;
  height: 46vw;
  right: -8vw;
  top: -6vw;
  background: radial-gradient(closest-side, var(--aurora-2), transparent 72%);
}

.blob-c {
  width: 60vw;
  height: 40vw;
  left: 24vw;
  bottom: -24vw;
  background: radial-gradient(closest-side, var(--aurora-3), transparent 70%);
}

.app-body {
  position: relative;
  z-index: 1;
  flex: 1;
  display: flex;
  min-height: 0;
}

.login-waiting-banner {
  margin: 0 0 12px;
  padding: 10px 16px;
  background: var(--accent-light);
  border: 1px solid rgba(0, 180, 216, 0.24);
  border-radius: var(--r-sm);
  color: var(--accent);
  font-size: 13px;
  font-weight: 600;
  text-align: center;
}

/* ===== 侧边栏 ===== */
.sidebar {
  position: relative;
  z-index: 1;
  width: 246px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  margin: 10px 0 10px 10px;
  border-radius: var(--r-xl);
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  box-shadow: var(--shadow-md-raised);
  overflow: hidden;
}

/* 窗口操作栏 */
.window-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px 4px;
}

.window-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  background: var(--surface);
  border: 1px solid var(--glass-border);
  border-radius: 10px;
  box-shadow: var(--shadow-soft-flat);
}

.win-btn {
  width: 26px;
  height: 24px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background var(--t-smooth), color var(--t-smooth), transform var(--t-press);
}

.win-btn:hover {
  background: rgba(148, 163, 184, 0.20);
  color: var(--text-primary);
}

.win-btn:active {
  transform: scale(0.92);
}

.win-btn.close:hover {
  background: #ef4444;
  color: #fff;
}

/* 主题切换钮 */
.theme-toggle {
  width: 30px;
  height: 30px;
  border-radius: 9px;
  background: var(--surface);
  border: 1px solid var(--glass-border);
  box-shadow: var(--shadow-soft-flat);
}

.theme-toggle:hover {
  background: var(--surface-raised);
}

.sidebar-header {
  padding: 6px 18px 18px;
}

.logo {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}

.logo-icon {
  width: 38px;
  height: 38px;
  background: var(--accent-grad);
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  color: #fff;
  box-shadow: 0 6px 16px rgba(0, 180, 216, 0.32), inset 0 1px 0 rgba(255, 255, 255, 0.35);
}

.logo-text {
  font-family: 'Nunito', 'Inter', sans-serif;
  font-size: 20px;
  font-weight: 800;
  letter-spacing: 0.2px;
  background: var(--accent-grad);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.logo-desc {
  font-size: 12px;
  color: var(--text-muted);
  margin-left: 50px;
}

.nav-menu {
  flex: 1;
  overflow-y: auto;
  padding: 4px 12px;
}

.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 11px 14px;
  border-radius: 12px;
  cursor: pointer;
  transition: background var(--t-smooth), color var(--t-smooth), box-shadow var(--t-smooth), transform var(--t-press);
  color: var(--text-secondary);
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 6px;
  background: transparent;
  outline: none;
}

.nav-item:hover {
  color: var(--text-primary);
  background: var(--surface);
}

.nav-item:active {
  transform: scale(0.985);
}

.nav-item:focus-visible {
  box-shadow: 0 0 0 2px var(--accent-light);
}

.nav-item.active {
  color: var(--accent);
  background: var(--surface-raised);
  font-weight: 700;
  box-shadow: var(--shadow-soft-flat);
}

.nav-item.active::before {
  content: '';
  position: absolute;
  left: 4px;
  top: 50%;
  width: 3px;
  height: 18px;
  border-radius: var(--r-pill);
  background: var(--accent-grad);
  transform: translateY(-50%);
}

.nav-icon {
  display: inline-flex;
  align-items: center;
}

.nav-badge {
  margin-left: auto;
  background: var(--accent-grad);
  color: #fff;
  font-size: 11px;
  padding: 2px 8px;
  border-radius: var(--r-pill);
  font-weight: 700;
  box-shadow: 0 3px 10px rgba(0, 180, 216, 0.35);
}

.sidebar-footer {
  padding: 12px;
}

.status-card {
  background: var(--surface);
  border: 1px solid var(--glass-border);
  border-radius: var(--r-md);
  padding: 14px;
  box-shadow: var(--shadow-soft-inset);
}

.status-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 10px;
}

.status-item:last-child {
  margin-bottom: 0;
}

.status-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.status-value {
  font-size: 13px;
  font-weight: 600;
}

.status-value.on {
  color: var(--success);
}

.status-value.off {
  color: var(--text-muted);
}

.status-value.success {
  color: var(--success);
}

/* ===== 主内容区 ===== */
.main-content {
  position: relative;
  z-index: 1;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  padding: 10px;
  overflow: hidden;
}

.top-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  padding: 16px 20px;
  margin-bottom: 12px;
  flex-shrink: 0;
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-soft-raised);
}

.top-bar-left {
  min-width: 0;
}

.page-title {
  font-size: 21px;
  font-weight: 800;
  color: var(--text-primary);
  margin-bottom: 3px;
  letter-spacing: -0.01em;
}

.page-subtitle {
  font-size: 13px;
  color: var(--text-secondary);
}

.top-bar-right {
  display: flex;
  gap: 10px;
  flex-shrink: 0;
}

.content-area {
  flex: 1;
  overflow-y: auto;
  padding: 2px 2px 6px;
  background: transparent;
  border: none;
  box-shadow: none;
}

/* 顶部通知 - 玻璃药丸 + 状态色 */
.top-toast {
  position: fixed;
  top: 20px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 2000;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 11px 20px;
  border-radius: var(--r-pill);
  background: var(--glass-bg-strong);
  border: 1px solid var(--glass-border);
  font-weight: 600;
  font-size: 13px;
  box-shadow: var(--shadow-md-raised);
  animation: toast-in 0.26s var(--t-lift);
}

.top-toast::before {
  content: '';
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.top-toast.success {
  color: var(--success);
}

.top-toast.success::before {
  background: var(--success);
  box-shadow: 0 0 8px var(--success);
}

.top-toast.error {
  color: var(--danger);
}

.top-toast.error::before {
  background: var(--danger);
  box-shadow: 0 0 8px var(--danger);
}

@keyframes toast-in {
  from { opacity: 0; transform: translate(-50%, -10px); }
  to { opacity: 1; transform: translate(-50%, 0); }
}

/* 刷新按钮图标旋转动画(:deep 穿透到 Icon 根 svg) */
:deep(.spinning) {
  animation: app-spin 0.8s linear infinite;
}

@keyframes app-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
