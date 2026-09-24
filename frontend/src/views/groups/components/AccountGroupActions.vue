<script setup lang="ts">
import type { AccountGroup } from '@/api'
import { BaseIconButton, BaseMenuItem, BasePopover } from '@codex-proxy/ui'

import { MoreHorizontal, Pencil, Power, Trash2 } from '@lucide/vue'

defineProps<{
  group: AccountGroup
  updatingStatus: boolean
  deleting: boolean
}>()
const emit = defineEmits<{
  edit: [group: AccountGroup]
  toggle: [group: AccountGroup]
  delete: [group: AccountGroup]
}>()
</script>

<template>
  <div class="flex items-center gap-0.5">
    <BaseIconButton
      variant="ghost"
      size="sm"
      label="编辑分组"
      @click.stop="emit('edit', group)"
    >
      <Pencil class="size-3.5 text-cp-link" />
    </BaseIconButton>
    <BaseIconButton
      variant="ghost"
      size="sm"
      label="删除分组"
      :disabled="deleting"
      @click.stop="emit('delete', group)"
    >
      <Trash2 class="size-3.5 text-cp-error" />
    </BaseIconButton>

    <BasePopover placement="bottom-end">
      <template #trigger="{ open }">
        <BaseIconButton variant="ghost" size="sm" label="更多操作" :pressed="open">
          <MoreHorizontal class="size-4" />
        </BaseIconButton>
      </template>
      <template #default="{ close }">
        <div class="w-44 p-1.5">
          <BaseMenuItem :loading="updatingStatus" @click.stop="(close(), emit('toggle', group))">
            <template #icon>
              <Power class="size-3.5" :class="group.enabled ? 'text-cp-warning' : 'text-cp-success'" />
            </template>
            {{ group.enabled ? '禁用分组' : '启用分组' }}
          </BaseMenuItem>
        </div>
      </template>
    </BasePopover>
  </div>
</template>
