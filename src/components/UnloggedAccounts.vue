<template>
  <div class="unlogged-page">
    <div class="unlogged-head">
      <div class="unlogged-title">
        实例未登录（{{ list.length }}）
        <span class="unlogged-scan">最近扫描 {{ scanText }}</span>
      </div>
      <button class="btn btn-sm btn-outline" :disabled="scanning" @click="rescan">
        <Icon name="arrow-path" :size="14" :class="{ 'unlogged-spin': scanning }" />
        <span>{{ scanning ? '扫描中…' : '重新扫描' }}</span>
      </button>
    </div>

    <div class="accounts-grid" v-if="list.length > 0">
      <AccountCard
        v-for="account in list"
        :key="account.id"
        :account="account"
        :hide-checkin="true"
        @toggle="handleToggle"
        @delete="handleDelete"
        @edit="handleEdit"
        @notify="handleNotify"
      />
    </div>

    <div class="empty-state" v-else>
      <Icon name="inbox" :size="48" class="empty-state-icon" />
      <p class="empty-state-text">当前没有实例未登录的账号</p>
      <p class="empty-state-desc">最近扫描 {{ scanText }}</p>
      <button class="btn btn-primary" style="margin-top: 20px" :disabled="scanning" @click="rescan">
        <Icon name="arrow-path" :size="16" />
        <span>重新扫描</span>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onActivated, onDeactivated, onMounted, ref, watch } from 'vue'
import { useAppStore } from '../stores/app'
import AccountCard from './AccountCard.vue'
import Icon from './Icon.vue'

const store = useAppStore()
const emit = defineEmits(['notify'])

const scanning = ref(false)
const lastScan = ref<Date | null>(null)

// 实例登录态失效的账号(instanceLoggedIn === false)
const list = computed(() => store.accounts.filter((a) => a.instanceLoggedIn === false))

const scanText = computed(() =>
  lastScan.value
    ? lastScan.value.toLocaleTimeString('zh-CN', { hour12: false })
    : '—'
)

// 账号数据变化即视为一次新扫描结果(扫描失败时 store 内部保留旧值,不清空列表)
watch(
  () => store.accounts,
  () => {
    lastScan.value = new Date()
  },
  { deep: true }
)

async function rescan() {
  if (scanning.value) return
  scanning.value = true
  try {
    await store.fetchAccounts()
    lastScan.value = new Date()
  } finally {
    scanning.value = false
  }
}

onMounted(() => {
  lastScan.value = new Date()
})

// 该页也需要实例状态:激活时启轮询,切走即停
onActivated(() => store.startInstanceStatePolling())
onDeactivated(() => store.stopInstanceStatePolling())

function handleNotify(message: string, type?: 'success' | 'error') {
  emit('notify', message, type)
}

async function handleToggle(id: string, enabled: boolean) {
  await store.updateAccount(id, { enabled })
}

async function handleDelete(id: string) {
  if (confirm('确定要删除这个账号吗？此操作不可恢复。')) {
    await store.deleteAccount(id)
  }
}

function handleEdit(id: string, newName?: string) {
  const account = store.accounts.find((item) => item.id === id)
  if (!account) return
  if (newName) {
    void store.updateAccount(id, { name: newName })
    return
  }
  const name = window.prompt('请输入账号名称', account.name)?.trim()
  if (!name || name === account.name) return
  void store.updateAccount(id, { name })
}
</script>

<style scoped>
.unlogged-page {
  max-width: 100%;
}

.unlogged-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 18px;
}

.unlogged-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-primary);
}

.unlogged-scan {
  margin-left: 10px;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
}

.unlogged-spin {
  animation: unlogged-spin 1s linear infinite;
}

@keyframes unlogged-spin {
  to {
    transform: rotate(360deg);
  }
}

.accounts-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 16px;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 60px 20px;
  text-align: center;
}

.empty-state-icon {
  color: var(--text-muted);
  margin-bottom: 16px;
}

.empty-state-text {
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 8px;
}

.empty-state-desc {
  font-size: 13px;
  color: var(--text-secondary);
}
</style>
