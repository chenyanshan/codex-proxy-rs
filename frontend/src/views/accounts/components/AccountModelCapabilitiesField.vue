<script setup lang="ts">
import type { ApiKeyConfiguration, ModelPresentationOverride } from '@/api'

import { BaseCheckbox, BaseEmpty, BaseFormItem, BaseScrollbar, BaseSwitch } from '@codex-proxy/ui'
import { computed, ref, watch } from 'vue'
import { getAccountModels } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

const props = withDefaults(defineProps<{
  accountId?: string
  disabled?: boolean
}>(), { disabled: false })

const model = defineModel<ApiKeyConfiguration['modelPresentationOverrides']>({ required: true })
const catalog = ref<Array<{ id: string, label: string }>>([])
const request = useRequestState()
const { loading, error } = request

const models = computed(() => {
  const entries = new Map(catalog.value.map(item => [item.id, item]))
  for (const id of Object.keys(model.value ?? {})) {
    if (!entries.has(id))
      entries.set(id, { id, label: id })
  }
  return [...entries.values()]
})

function overrideFor(id: string): ModelPresentationOverride {
  return model.value?.[id] ?? { imageInput: false, imageDetailOriginal: false }
}

function update(id: string, patch: Partial<ModelPresentationOverride>) {
  if (props.disabled)
    return
  const next = { ...(model.value ?? {}) }
  const value = { ...overrideFor(id), ...patch }
  if (!value.imageInput)
    value.imageDetailOriginal = false
  if (!value.imageInput && !value.imageDetailOriginal)
    delete next[id]
  else
    next[id] = value
  model.value = next
}

async function load() {
  if (!props.accountId)
    return
  const requestId = request.start()
  try {
    const result = await getAccountModels({ accountId: props.accountId }, { signal: request.signal, silent: true })
    if (request.isCurrent(requestId))
      catalog.value = result.models
  }
  catch (cause) {
    request.fail(requestId, cause)
  }
  finally {
    request.finish(requestId)
  }
}

watch(() => props.accountId, () => {
  request.invalidate()
  catalog.value = []
  error.value = ''
  void load()
}, { immediate: true })
</script>

<template>
  <BaseFormItem label="模型输入能力">
    <template #extra>
      <span class="text-cp-xs text-cp-text-quaternary">控制 /v1/models 展示，不会替上游验证能力</span>
    </template>
    <BaseScrollbar v-if="models.length" max-height="15rem" class="min-w-0 -m-1">
      <div class="grid gap-2 p-1" role="group" aria-label="模型输入能力">
        <div
          v-for="item in models"
          :key="item.id"
          role="group"
          :aria-label="item.id"
          class="grid gap-2 rounded-cp bg-cp-fill-quaternary px-3 py-2.5 sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center"
        >
          <span class="truncate text-cp-sm text-cp-text" :title="item.id">{{ item.id }}</span>
          <BaseCheckbox
            :model-value="overrideFor(item.id).imageInput"
            label="图片输入"
            show-label
            :disabled="disabled"
            @update:model-value="update(item.id, { imageInput: $event })"
          />
          <BaseSwitch
            :model-value="overrideFor(item.id).imageDetailOriginal"
            label="原图 detail"
            :disabled="disabled || !overrideFor(item.id).imageInput"
            @update:model-value="update(item.id, { imageDetailOriginal: $event })"
          />
        </div>
      </div>
    </BaseScrollbar>
    <p v-if="error && models.length" class="m-0 text-cp-xs text-cp-error" role="alert">
      模型目录加载失败，当前仅显示已保存的能力声明
    </p>
    <p v-else-if="loading" class="m-0 py-3 text-center text-cp-sm text-cp-text-tertiary" role="status">
      加载模型中…
    </p>
    <BaseEmpty v-else-if="!error" title="暂无模型目录" description="先刷新上游模型目录后再配置能力" size="sm" surface="none" />
    <BaseEmpty v-else title="模型目录加载失败" description="保存连接设置后重试" size="sm" surface="none" role="alert" />
  </BaseFormItem>
</template>
