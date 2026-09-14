<script setup lang="ts">
import type { Account } from '@/api'
import { formatDateTime } from '@/utils/date'

defineProps<{ subscription: Account['subscription'] }>()
</script>

<template>
  <div v-if="subscription" class="grid gap-1 text-cp-text-secondary" :title="`查询时间（北京时间）：${formatDateTime(subscription.observedAt, '—', 'Asia/Shanghai')}`">
    <time :datetime="subscription.expiresAt" class="whitespace-nowrap font-mono text-cp-sm tabular-nums">
      {{ formatDateTime(subscription.expiresAt, '—', 'Asia/Shanghai') }}
    </time>
    <span class="text-cp-xs text-cp-text-tertiary">
      {{ subscription.willRenew === true ? '本期结束 · 自动续费' : subscription.willRenew === false ? '不自动续费' : '续费状态未知' }}
    </span>
  </div>
  <span v-else class="text-cp-sm text-cp-text-tertiary">未知</span>
</template>
