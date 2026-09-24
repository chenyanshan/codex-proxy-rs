<script setup lang="ts">
import type { useAccountConnectionTest } from '../composables/useAccountConnectionTest'

import { BaseButton, BaseIconButton, BaseModal, BaseSelect } from '@codex-proxy/ui'
import { Fingerprint, RefreshCw } from '@lucide/vue'
import AccountIdentityCell from './AccountIdentityCell.vue'
import AccountStatusBadge from './AccountStatusBadge/index.vue'

type ConnectionTest = ReturnType<typeof useAccountConnectionTest>

defineProps<{
  account: ConnectionTest['testingAccount']['value']
  status: ConnectionTest['connectionTestStatus']['value']
  model: string
  logs: ConnectionTest['connectionTestLogs']['value']
  error: string
  startedAt: string
  finishedAt: string
  durationMs: number | null
  loadingModels: boolean
  refreshingModels: boolean
  modelOptions: ConnectionTest['connectionTestModelOptions']['value']
  statusView: ConnectionTest['connectionTestStatusView']['value']
  upstreamResponseModel: string
  modelAttributionStatus: ConnectionTest['modelAttributionStatus']['value']
  modelAttributionProgress: number
  modelAttributionResult: ConnectionTest['modelAttributionResult']['value']
  modelAttributionError: string
  modelAttributionResponseModels: string[]
}>()

const emit = defineEmits<{
  test: []
  attribute: []
  refreshModels: []
}>()
const open = defineModel<boolean>({ default: false })
const selectedModel = defineModel<string>('selectedModel', { required: true })

function connectionLogClass(tone: string) {
  if (tone === 'success')
    return 'text-cp-success-text'
  if (tone === 'danger')
    return 'text-cp-error-text'
  if (tone === 'info')
    return 'text-cp-info-text'
  return 'text-cp-text-secondary'
}

function formatPercent(value: number) {
  return `${(value * 100).toFixed(1)}%`
}
</script>

<template>
  <BaseModal
    v-model="open"
    title="测试连接"
    description="验证账号凭据、身份绑定与上游模型端点是否可用"
    tone="info"
    size="lg"
  >
    <div v-if="account" class="flex flex-col gap-4">
      <section
        class="flex items-center justify-between gap-4 rounded-cp-card bg-cp-fill-quaternary px-4 py-3"
      >
        <AccountIdentityCell :account="account" size="lg" show-plan />
        <AccountStatusBadge
          :status="account.status"
          :error-reason="account.errorReason"
          :error-message="account.errorMessage"
          :rate-limited-until="account.quota.rateLimitedUntil"
          :rate-limit-reason="account.quota.rateLimitReason"
          :recovery-probe-required="account.quota.recoveryProbeRequired"
          :next-refresh-at="account.nextRefreshAt"
          variant="pill"
        />
      </section>

      <section class="rounded-cp-card bg-cp-fill-quaternary px-4 py-3">
        <div class="grid gap-2">
          <div class="flex min-h-8 items-center justify-between gap-3">
            <span class="text-cp-sm font-heavy text-cp-text-quaternary">
              测试模型
            </span>
            <BaseIconButton
              variant="ghost"
              size="sm"
              label="刷新上游模型"
              :loading="refreshingModels"
              :disabled="status === 'running' || modelAttributionStatus === 'running' || loadingModels"
              @click="emit('refreshModels')"
            >
              <template #loading>
                <RefreshCw class="size-3.5 animate-spin motion-reduce:animate-none" />
              </template>
              <RefreshCw class="size-3.5" />
            </BaseIconButton>
          </div>
          <BaseSelect
            v-model="selectedModel"
            aria-label="测试模型"
            :options="modelOptions"
            :disabled="status === 'running' || modelAttributionStatus === 'running' || loadingModels || refreshingModels"
            :placeholder="loadingModels ? '加载模型中...' : '选择上游模型'"
            empty-text="上游没有返回模型"
          />
        </div>
      </section>

      <section class="rounded-cp-card bg-cp-fill-quaternary p-4">
        <div class="flex items-start justify-between gap-4">
          <div class="flex min-w-0 items-start gap-3">
            <span
              class="inline-flex size-10 shrink-0 items-center justify-center rounded-lg"
              :class="statusView.badge"
            >
              <component
                :is="statusView.icon"
                class="size-5"
                :class="[statusView.iconClass, status === 'running' ? 'animate-pulse' : '']"
              />
            </span>
            <div class="min-w-0">
              <p class="m-0 text-[16px] font-heavy text-cp-text">
                {{ statusView.label }}
              </p>
              <p
                class="mt-1.5 mb-0 text-cp leading-normal font-emphasis text-cp-text-secondary"
              >
                {{ statusView.description }}
              </p>
            </div>
          </div>
          <span
            class="inline-flex h-7 shrink-0 items-center rounded-full px-2.5 text-cp-sm font-heavy"
            :class="statusView.badge"
          >
            {{ status === 'running' ? '检测中' : statusView.label }}
          </span>
        </div>

        <div class="mt-4 grid gap-3 sm:grid-cols-3">
          <div class="rounded-lg bg-cp-bg-container px-3 py-2.5">
            <p class="m-0 text-cp-xs font-heavy text-cp-text-quaternary">
              开始时间
            </p>
            <p class="mt-1.5 mb-0 font-mono text-cp-sm font-emphasis text-cp-text">
              {{ startedAt || '-' }}
            </p>
          </div>
          <div class="rounded-lg bg-cp-bg-container px-3 py-2.5">
            <p class="m-0 text-cp-xs font-heavy text-cp-text-quaternary">
              完成时间
            </p>
            <p class="mt-1.5 mb-0 font-mono text-cp-sm font-emphasis text-cp-text">
              {{ finishedAt || '-' }}
            </p>
          </div>
          <div class="rounded-lg bg-cp-bg-container px-3 py-2.5">
            <p class="m-0 text-cp-xs font-heavy text-cp-text-quaternary">
              响应耗时
            </p>
            <p class="mt-1.5 mb-0 font-mono text-cp-sm font-emphasis text-cp-text">
              {{ durationMs !== null ? `${durationMs}ms` : '-' }}
            </p>
          </div>
        </div>

        <div class="mt-3 rounded-lg bg-cp-bg-container px-3 py-2.5">
          <p class="m-0 text-cp-xs font-heavy text-cp-text-quaternary">
            测试模型
          </p>
          <p
            class="mt-1.5 mb-0 truncate font-mono text-cp-sm font-emphasis text-cp-text"
            :title="model || '-'"
          >
            {{ model || '-' }}
          </p>
          <p
            v-if="upstreamResponseModel && upstreamResponseModel !== model"
            class="mt-1 mb-0 truncate font-mono text-cp-sm font-emphasis text-cp-text-secondary"
            :title="upstreamResponseModel"
          >
            ↳ {{ upstreamResponseModel }}
          </p>
        </div>

        <div class="mt-3 rounded-lg bg-cp-bg-container px-3 py-2.5">
          <p class="m-0 text-cp-xs font-heavy text-cp-text-quaternary">
            事件轨迹
          </p>
          <div class="pt-2">
            <div v-if="logs.length === 0" class="text-cp-sm font-emphasis text-cp-text-quaternary">
              -
            </div>
            <div v-else class="flex flex-col gap-1.5">
              <div
                v-for="item in logs"
                :key="item.key"
                class="grid grid-cols-[54px_minmax(0,1fr)] gap-2 text-cp-sm leading-[1.45] font-emphasis"
              >
                <span class="font-mono text-cp-text-quaternary">{{ item.time }}</span>
                <div class="min-w-0">
                  <p
                    class="m-0 wrap-break-word"
                    :class="connectionLogClass(item.tone)"
                  >
                    {{ item.text }}
                  </p>
                  <div v-if="item.detail" class="mt-2 rounded-lg bg-cp-fill-quaternary px-3 py-2">
                    <p
                      v-if="item.tone === 'danger'"
                      class="mt-0 mb-2 text-cp-xs font-heavy text-cp-text-quaternary"
                    >
                      原始诊断
                    </p>
                    <pre
                      class="m-0 whitespace-pre-wrap wrap-break-word font-mono text-cp-xs leading-[1.6] font-emphasis text-cp-text"
                      v-text="item.detail"
                    />
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section class="rounded-cp-card bg-cp-fill-quaternary p-4">
        <div class="flex items-start gap-3">
          <span class="inline-flex size-9 shrink-0 items-center justify-center rounded-lg bg-cp-info-container text-cp-info-on-container">
            <Fingerprint class="size-4.5" />
          </span>
          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div>
                <p class="m-0 text-cp-sm font-heavy text-cp-text">
                  主动探测模型归因
                </p>
                <p class="mt-1 mb-0 text-cp-xs leading-normal font-emphasis text-cp-text-secondary">
                  发送 3 次长数字序列探针，与 ModelTrace 指纹库进行闭集相似度比较
                </p>
              </div>
              <span
                v-if="modelAttributionStatus !== 'idle'"
                class="inline-flex h-7 items-center rounded-full px-2.5 text-cp-xs font-heavy"
                :class="modelAttributionStatus === 'success'
                  ? 'bg-cp-success-container text-cp-success-on-container'
                  : modelAttributionStatus === 'error'
                    ? 'bg-cp-error-container text-cp-error-on-container'
                    : 'bg-cp-info-container text-cp-info-on-container'"
              >
                {{ modelAttributionStatus === 'running'
                  ? `探测中 ${modelAttributionProgress}/3`
                  : modelAttributionStatus === 'success' ? '归因完成' : '归因失败' }}
              </span>
            </div>

            <div v-if="modelAttributionStatus === 'running'" class="mt-3 h-1.5 overflow-hidden rounded-full bg-cp-bg-container">
              <div
                class="h-full rounded-full bg-cp-info transition-[width]"
                :style="{ width: `${(modelAttributionProgress / 3) * 100}%` }"
              />
            </div>

            <div v-if="modelAttributionResult" class="mt-3 grid gap-2">
              <div class="rounded-lg bg-cp-bg-container px-3 py-2.5">
                <p class="m-0 text-cp-xs font-heavy text-cp-text-quaternary">
                  最接近候选模型
                </p>
                <div class="mt-1.5 flex flex-wrap items-baseline justify-between gap-2">
                  <p class="m-0 font-mono text-cp-sm font-heavy text-cp-text">
                    {{ modelAttributionResult.predictionName }}
                  </p>
                  <span class="font-mono text-cp-sm font-heavy text-cp-info-text">
                    {{ formatPercent(modelAttributionResult.probability) }}
                  </span>
                </div>
                <p class="mt-1 mb-0 text-cp-xs font-emphasis text-cp-text-quaternary">
                  有效探针 {{ modelAttributionResult.usedOutputs }}/3 · 交叉验证准确率 {{ formatPercent(modelAttributionResult.cvAccuracy) }}
                </p>
              </div>

              <div
                v-for="candidate in modelAttributionResult.results.slice(0, 3)"
                :key="candidate.model"
                class="grid grid-cols-[minmax(0,1fr)_52px] items-center gap-3"
              >
                <div class="min-w-0">
                  <div class="mb-1 flex items-center justify-between gap-2 text-cp-xs font-emphasis">
                    <span class="truncate text-cp-text-secondary">{{ candidate.displayName }}</span>
                    <span class="font-mono text-cp-text-quaternary">{{ formatPercent(candidate.probability) }}</span>
                  </div>
                  <div class="h-1.5 overflow-hidden rounded-full bg-cp-bg-container">
                    <div
                      class="h-full rounded-full bg-cp-info"
                      :style="{ width: `${Math.max(candidate.probability * 100, 1)}%` }"
                    />
                  </div>
                </div>
                <span class="text-right font-mono text-cp-xs text-cp-text-quaternary">
                  {{ formatPercent(candidate.profileSimilarity) }}
                </span>
              </div>

              <p v-if="modelAttributionResponseModels.length" class="m-0 text-cp-xs font-emphasis text-cp-text-secondary">
                上游自报模型：<span class="font-mono text-cp-text">{{ modelAttributionResponseModels.join(', ') }}</span>
              </p>
            </div>

            <p v-if="modelAttributionError" role="alert" class="mt-3 mb-0 text-cp-sm font-emphasis text-cp-error-text">
              {{ modelAttributionError }}
            </p>

            <p class="mt-3 mb-0 rounded-cp bg-cp-warning-container px-3 py-2 text-cp-xs leading-normal font-emphasis text-cp-warning-on-container">
              结果只表示与当前指纹库候选模型的相似度，不是上游身份认证；未收录模型仍会匹配到最接近候选。探测会产生真实请求并消耗额度。
            </p>
          </div>
        </div>
      </section>
    </div>

    <template #footer>
      <BaseButton variant="secondary" @click="open = false">
        关闭
      </BaseButton>
      <BaseButton
        variant="secondary"
        :loading="modelAttributionStatus === 'running'"
        :disabled="!account || status === 'running' || loadingModels || refreshingModels || !selectedModel"
        @click="emit('attribute')"
      >
        主动探测模型归因
      </BaseButton>
      <BaseButton
        variant="primary"
        :loading="status === 'running'"
        :disabled="!account || modelAttributionStatus === 'running' || loadingModels || refreshingModels || !selectedModel"
        @click="emit('test')"
      >
        {{ logs.length > 0 || error ? '重新测试' : '开始测试' }}
      </BaseButton>
    </template>
  </BaseModal>
</template>
