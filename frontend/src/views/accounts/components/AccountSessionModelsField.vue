<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { getAccountModels } from '@/api'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCheckbox from '@/components/base/BaseCheckbox.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseScrollbar from '@/components/base/BaseScrollbar.vue'
import { useRequestState } from '@/composables/useRequestState'
import { accountModelIdError } from '../utils/modelAccess'

const props = defineProps<{ accountId: string, disabled: boolean }>()
const selected = defineModel<string[]>({ required: true })
const catalog = ref<string[]>([])
const manual = ref('')
const inputError = ref('')
const request = useRequestState()
const models = computed(() => [...new Set([...catalog.value, ...selected.value])])
function select(id: string, checked: boolean) {
  if (props.disabled)
    return
  if (checked && selected.value.length >= 32) {
    inputError.value = '最多选择 32 个重写模型'
    return
  }
  selected.value = checked ? [...new Set([...selected.value, id])] : selected.value.filter(model => model !== id)
  inputError.value = ''
}
function add() {
  const id = manual.value.trim()
  inputError.value = accountModelIdError(id) ?? ''
  if (!inputError.value) {
    select(id, true)
    if (!inputError.value)
      manual.value = ''
  }
}
async function load() {
  const id = request.start()
  try {
    const result = await getAccountModels({ accountId: props.accountId }, { signal: request.signal, silent: true })
    if (request.isCurrent(id))
      catalog.value = result.models.map(model => model.id)
  }
  catch (error) {
    request.fail(id, error)
  }
  finally {
    request.finish(id)
  }
}
watch(() => props.accountId, () => {
  request.invalidate()
  catalog.value = []
  void load()
}, { immediate: true })
</script>

<template>
  <BaseFormItem label="State 重写模型" description="自动使用全局动态代理。每个模型独立发送 hi、提取并缓存 State，互不混用。请选择实际的上游模型 ID（最多 32 个）。">
    <div class="grid gap-3">
      <BaseScrollbar max-height="12rem">
        <div class="grid grid-cols-2 gap-2" role="group" aria-label="选择重写模型">
          <BaseCheckbox v-for="model in models" :key="model" :label="model" :title="model" show-label :model-value="selected.includes(model)" :disabled="disabled" @update:model-value="select(model, $event)" />
        </div>
      </BaseScrollbar>
      <div class="flex gap-2">
        <BaseInput v-model="manual" class="min-w-0 flex-1" aria-label="自定义重写模型" placeholder="输入其他模型 ID" :disabled="disabled" @keydown.enter.prevent="add" />
        <BaseButton variant="secondary" :disabled="disabled || !manual.trim()" @click="add">
          添加
        </BaseButton>
      </div>
      <p v-if="inputError" role="alert" class="m-0 text-cp-sm text-cp-error-text">
        {{ inputError }}
      </p>
      <p v-if="request.loading.value" role="status" class="m-0 text-cp-sm text-cp-text-secondary">
        正在加载账户模型…
      </p>
      <p v-if="request.error.value" class="m-0 text-cp-sm text-cp-warning-text">
        模型目录加载失败，可以手动填写模型 ID。
      </p>
    </div>
  </BaseFormItem>
</template>
