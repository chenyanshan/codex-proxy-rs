<script setup lang="ts">
import { BaseCard, BaseForm, BaseFormItem, BaseInput, BaseSwitch } from '@codex-proxy/ui'
import { Clock, Sparkles } from '@lucide/vue'

const enabled = defineModel<boolean>('enabled', { required: true })
const scheduleTime = defineModel<string>('scheduleTime', { required: true })
const model = defineModel<string>('model', { required: true })
</script>

<template>
  <BaseCard
    title="账号预激活（Warmup）"
    description="每日定时向 OAuth 账号发起极微小请求，提前并行启动 5h 额度重置倒计时，提高全天可用额度"
  >
    <BaseForm class="max-w-6xl sm:grid-cols-2">
      <BaseSwitch
        v-model="enabled"
        class="col-span-full justify-self-start"
        label="启用每日预激活"
        show-label
      />

      <BaseFormItem
        label="每日激活时间"
        description="支持 HH:MM 格式，多个时间点用英文逗号分隔（如 08:00 或 08:00,13:00，基于北京时间 UTC+8）"
      >
        <BaseInput
          v-model="scheduleTime"
          :disabled="!enabled"
          aria-label="每日激活时间"
          placeholder="08:00"
        >
          <template #prefix>
            <Clock class="size-4" />
          </template>
        </BaseInput>
      </BaseFormItem>

      <BaseFormItem
        label="预激活模型"
        description="触发预激活所请求的上游模型，留空默认使用 gpt-5.4-mini"
      >
        <BaseInput
          v-model="model"
          :disabled="!enabled"
          aria-label="预激活模型"
          placeholder="gpt-5.4-mini"
        >
          <template #prefix>
            <Sparkles class="size-4" />
          </template>
        </BaseInput>
      </BaseFormItem>
    </BaseForm>
  </BaseCard>
</template>
