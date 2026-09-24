<script setup lang="ts">
import type { KeyUsageModel } from '@/api/modules/key-usage'
import { BaseCard, BaseTable, defineTableColumns } from '@codex-proxy/ui'
import { formatInteger } from '@/utils/number'
import { money } from '../utils/format'

defineProps<{ models: KeyUsageModel[] }>()

const columns = defineTableColumns<KeyUsageModel>([
  { key: 'model', label: '模型', kind: 'custom', size: 'xl' },
  { key: 'requests', label: '请求', kind: 'numeric', size: 'sm' },
  { key: 'totalTokens', label: 'TOKEN', kind: 'numeric', size: 'lg' },
  { key: 'costUsd', label: '估算费用 USD', kind: 'numeric', size: 'lg' },
])
</script>

<template>
  <BaseCard title="按模型用量" description="当前密钥在所选范围内的请求、Token 与估算费用">
    <div class="h-70 min-h-0 overflow-hidden">
      <BaseTable :columns="columns" :rows="models" row-key="model" class="min-w-0" empty-text="所选条件下暂无模型用量">
        <template #model="{ row }">
          <code class="block max-w-full truncate font-mono text-cp-sm font-heavy text-cp-text">{{ row.model }}</code>
        </template>
        <template #requests="{ row }">
          <span class="font-mono tabular-nums">{{ formatInteger(row.requests) }}</span>
        </template>
        <template #totalTokens="{ row }">
          <span class="font-mono tabular-nums">{{ formatInteger(row.totalTokens) }}</span>
        </template>
        <template #costUsd="{ row }">
          <span class="grid justify-items-end gap-1 font-mono tabular-nums">
            <strong>{{ money(row.costUsd) }}</strong>
            <small v-if="row.costIncomplete" class="text-cp-xs text-cp-text-tertiary">部分费用缺失</small>
          </span>
        </template>
      </BaseTable>
    </div>
  </BaseCard>
</template>
