<script setup lang="ts">
import type { AccountRow } from '../constants'
import type { SessionStateRefresh } from '@/api'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import { formatDateTime } from '@/utils/date'

defineProps<{
  account: AccountRow | null
  result: SessionStateRefresh | null
  loading: boolean
  error: string
}>()
const emit = defineEmits<{ retry: [account: AccountRow] }>()
const open = defineModel<boolean>({ required: true })
</script>

<template>
  <BaseModal v-model="open" title="刷新 State" size="md" :dismissible="!loading">
    <div class="grid gap-4">
      <p class="m-0 text-cp text-cp-text-secondary">
        {{ account?.name }} · 通过动态代理向每个已选模型发送 hi 并提取 State，成功后有效期为 60 分钟。失败会自动重试，单个模型最多尝试 3 次。
      </p>
      <p v-if="loading" role="status" class="m-0 text-cp text-cp-text-secondary">
        正在刷新，请稍候…
      </p>
      <p v-if="error" role="alert" class="m-0 text-cp text-cp-error">
        {{ error }}
      </p>
      <div v-for="model in result?.models ?? (account?.sessionKeepaliveModels ?? []).map(model => ({ model, error: null, expireAt: null, refreshedAt: null }))" :key="model.model" class="grid gap-2 rounded-cp bg-cp-fill-quaternary p-4">
        <div class="flex items-center justify-between gap-3">
          <span class="font-mono text-cp font-medium text-cp-text">{{ model.model }}</span>
          <span class="text-cp-sm" :class="model.error ? 'text-cp-error' : model.expireAt ? 'text-cp-success' : 'text-cp-text-tertiary'">
            {{ model.error ? '刷新失败' : model.expireAt ? '已刷新' : loading ? '刷新中' : '未刷新' }}
          </span>
        </div>
        <p v-if="model.error" class="m-0 text-cp-sm text-cp-error">
          {{ model.error }}；仍有效的旧 State 保留至原到期时间。
        </p>
        <dl v-if="model.expireAt" class="m-0 grid gap-1 text-cp-sm text-cp-text-secondary">
          <div class="flex justify-between gap-2">
            <dt>刷新时间</dt><dd class="m-0">
              {{ formatDateTime(model.refreshedAt) }}
            </dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt>到期时间</dt><dd class="m-0">
              {{ formatDateTime(model.expireAt * 1000) }}
            </dd>
          </div>
        </dl>
      </div>
    </div>
    <template #footer>
      <BaseButton variant="secondary" :disabled="loading" @click="open = false">
        关闭
      </BaseButton>
      <BaseButton variant="primary" :loading="loading" :disabled="!account || !account.enabled || !account.enableSessionKeepalive" @click="account && emit('retry', account)">
        再次刷新
      </BaseButton>
    </template>
  </BaseModal>
</template>
