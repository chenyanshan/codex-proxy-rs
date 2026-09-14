<script setup lang="ts">
import type { ClientKeyUsage } from '@/api/modules/client-usage'
import { useEventListener } from '@vueuse/core'
import { computed, onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import { getClientKeyUsage } from '@/api/modules/client-usage'
import { ApiError } from '@/api/request'
import AppBrandMark from '@/components/AppBrandMark.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseSkeleton from '@/components/base/BaseSkeleton.vue'
import { useThemeStore } from '@/stores/modules/theme'
import { formatDateTime } from '@/utils/date'
import { formatInteger } from '@/utils/number'

const themeStore = useThemeStore()
const storageKey = 'codex-proxy-rs-client-usage-key'
const rememberedKey = shallowRef(readRememberedKey())
const key = shallowRef(rememberedKey.value)
const usage = shallowRef<ClientKeyUsage | null>(null)
const loading = shallowRef(false)
const errorMessage = shallowRef('')
let controller: AbortController | undefined
let sequence = 0

function readRememberedKey(fallback = '') {
  try {
    return localStorage.getItem(storageKey) ?? ''
  }
  catch {
    // 浏览器禁止存储时，仍允许当前页面查询。
    return fallback
  }
}

function rememberKey(value: string) {
  rememberedKey.value = value
  try {
    if (value)
      localStorage.setItem(storageKey, value)
    else
      localStorage.removeItem(storageKey)
  }
  catch {
    // 存储失败不改变查询结果，当前页面继续保留可用输入。
  }
}

const windows = computed(() => usage.value
  ? [
      { label: '日费用', used: usage.value.dailyUsedUsd, limit: usage.value.dailyLimitUsd, remaining: usage.value.dailyRemainingUsd, reset: usage.value.dailyResetsAt },
      { label: '七天费用', used: usage.value.weeklyUsedUsd, limit: usage.value.weeklyLimitUsd, remaining: usage.value.weeklyRemainingUsd, reset: usage.value.weeklyResetsAt },
    ]
  : [])

function resetResult() {
  // 同步失效请求序号，避免改 Key 或清空后旧响应重新填入结果。
  sequence++
  controller?.abort()
  controller = undefined
  loading.value = false
  usage.value = null
  errorMessage.value = ''
}

watch(key, resetResult, { flush: 'sync' })

async function queryUsage() {
  if (!key.value.trim() || loading.value)
    return
  resetResult()
  const currentSequence = sequence
  const submittedKey = key.value.trim()
  controller = new AbortController()
  loading.value = true
  try {
    const result = await getClientKeyUsage(submittedKey, controller.signal)
    if (currentSequence === sequence) {
      usage.value = result
      rememberKey(submittedKey)
    }
  }
  catch (error) {
    if (currentSequence === sequence)
      errorMessage.value = error instanceof ApiError ? error.message : '查询失败，请稍后重试'
  }
  finally {
    if (currentSequence === sequence) {
      loading.value = false
      controller = undefined
    }
  }
}

function clearKey() {
  rememberKey('')
  key.value = ''
  resetResult()
}

function leavePage() {
  key.value = ''
  resetResult()
}

onMounted(queryUsage)
onBeforeUnmount(leavePage)
// 往返缓存不会卸载组件；恢复成功记住的 Key 后重新查询，不展示离开前的旧结果。
useEventListener(window, 'pagehide', leavePage)
useEventListener(window, 'pageshow', (event) => {
  if (event.persisted) {
    rememberedKey.value = readRememberedKey(rememberedKey.value)
    key.value = rememberedKey.value
    resetResult()
    void queryUsage()
  }
})
</script>

<template>
  <main class="min-h-dvh bg-cp-bg-layout px-4 py-8 text-cp-text sm:py-12">
    <div class="mx-auto grid w-full max-w-3xl gap-6">
      <header class="flex flex-wrap items-center justify-between gap-4">
        <div class="flex items-center gap-3">
          <AppBrandMark class="size-10" />
          <div>
            <p class="m-0 text-sm text-cp-text-secondary">
              Codex Proxy RS
            </p>
            <h1 class="m-0 text-2xl font-semibold">
              查询 Key 用量
            </h1>
          </div>
        </div>
        <BaseButton variant="ghost" @click="themeStore.toggleTheme($event)">
          {{ themeStore.effectiveTheme === 'dark' ? '切换浅色' : '切换深色' }}
        </BaseButton>
      </header>

      <BaseCard as="form" class="grid gap-4" @submit.prevent="queryUsage">
        <span id="client-key-label" class="font-semibold">客户端 Key</span>
        <BaseInput
          id="client-key"
          v-model="key"
          type="password"
          autocomplete="off"
          autocapitalize="none"
          :spellcheck="false"
          placeholder="粘贴客户端 Key"
          aria-describedby="key-help"
          aria-labelledby="client-key-label"
        />
        <p id="key-help" class="m-0 text-sm leading-relaxed text-cp-text-secondary">
          无需管理员登录。当前浏览器记住上次查询成功的 Key，再次打开自动查询，清空可移除。
        </p>
        <div class="flex flex-wrap gap-3">
          <BaseButton type="submit" variant="primary" :loading="loading" :disabled="!key.trim() || loading">
            {{ loading ? '正在查询' : usage ? '刷新用量' : '查询用量' }}
          </BaseButton>
          <BaseButton :disabled="!key && !rememberedKey && !loading" @click="clearKey">
            清空
          </BaseButton>
        </div>
      </BaseCard>

      <div aria-live="polite" :aria-busy="loading">
        <BaseCard v-if="loading" class="grid gap-4" role="status">
          <span class="text-sm text-cp-text-secondary">正在读取已记录费用…</span>
          <BaseSkeleton class="h-8 w-1/2" />
          <BaseSkeleton class="h-20 w-full" />
        </BaseCard>
        <BaseCard v-else-if="errorMessage" role="alert">
          <p class="m-0 text-cp-error-text">
            {{ errorMessage }}
          </p>
        </BaseCard>
        <div v-else-if="usage" class="grid gap-4">
          <p class="m-0 text-sm text-cp-text-secondary">
            数据时间（北京时间）：<time :datetime="usage.asOf">{{ formatDateTime(usage.asOf, '—', usage.timezone) }}</time>
            · 币种 USD
          </p>
          <div class="grid gap-4 sm:grid-cols-2">
            <BaseCard v-for="window in windows" :key="window.label" :title="window.label">
              <p class="m-0 text-sm text-cp-text-secondary">
                已记录费用
              </p>
              <p class="my-2 break-all font-mono text-2xl tabular-nums" :class="window.remaining === '0' ? 'text-cp-error-text' : 'text-cp-text'">
                ${{ window.used }}
              </p>
              <p v-if="window.remaining === '0'" class="mt-0 mb-3 text-sm text-cp-error-text">
                已达到额度，仍可查询用量
              </p>
              <dl class="m-0 grid gap-3 text-sm">
                <div class="flex justify-between gap-3">
                  <dt class="shrink-0 text-cp-text-secondary">
                    额度
                  </dt>
                  <dd class="m-0 break-all text-right font-mono">
                    {{ window.limit === '0' ? '不限' : `$${window.limit}` }}
                  </dd>
                </div>
                <div class="flex justify-between gap-3">
                  <dt class="shrink-0 text-cp-text-secondary">
                    剩余
                  </dt>
                  <dd class="m-0 break-all text-right font-mono">
                    {{ window.remaining === null ? '不限' : `$${window.remaining}` }}
                  </dd>
                </div>
                <div class="grid gap-1">
                  <dt class="text-cp-text-secondary">
                    重置时间（北京时间）
                  </dt>
                  <dd class="m-0">
                    <time v-if="window.reset" :datetime="window.reset" class="font-mono">{{ formatDateTime(window.reset, '—', usage.timezone) }}</time>
                    <span v-else>下次使用时确定</span>
                  </dd>
                </div>
              </dl>
            </BaseCard>
          </div>
          <BaseCard title="请求限制" description="并发与每分钟请求上限">
            <dl class="m-0 grid gap-4 sm:grid-cols-2">
              <div>
                <dt class="text-sm text-cp-text-secondary">
                  最大并发
                </dt>
                <dd class="mx-0 mt-1 font-mono text-xl">
                  {{ usage.maxConcurrency === 0 ? '不限' : formatInteger(usage.maxConcurrency) }}
                </dd>
              </div>
              <div>
                <dt class="text-sm text-cp-text-secondary">
                  每分钟请求数（RPM）
                </dt>
                <dd class="mx-0 mt-1 font-mono text-xl">
                  {{ usage.requestsPerMinute === 0 ? '不限' : formatInteger(usage.requestsPerMinute) }}
                </dd>
              </div>
            </dl>
          </BaseCard>
        </div>
        <p v-else class="m-0 text-center text-sm text-cp-text-secondary">
          输入 Key 后查询日／七天费用与请求限制。
        </p>
      </div>

      <footer class="grid gap-3 text-sm leading-relaxed text-cp-text-secondary">
        <p class="m-0">
          日费用在北京时间零点重置。七天窗口从首次使用当日零点起算七天，并非自然周；尚未开始的窗口在下次使用时确定重置时间。费用以已记录数据为准。
        </p>
        <RouterLink to="/login" class="justify-self-start rounded-cp text-cp-link outline-none focus-visible:ring-2 focus-visible:ring-cp-control-outline">
          管理员登录
        </RouterLink>
      </footer>
    </div>
  </main>
</template>
