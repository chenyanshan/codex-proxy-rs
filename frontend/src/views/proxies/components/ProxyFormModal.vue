<script setup lang="ts">
import type { OutboundProxyRecord, OutboundProxyTest, RequestLocation } from '@/api'
import { BaseButton, BaseForm, BaseFormItem, BaseIconButton, BaseInput, BaseModal, BaseSwitch } from '@codex-proxy/ui'
import { Eye, EyeOff, Save, Wifi } from '@lucide/vue'
import { computed, shallowRef, watch } from 'vue'
import RequestLocationFields from '@/components/RequestLocationFields.vue'
import { formatDateTime } from '@/utils/date'
import { locationDetectionWarning } from '../utils/location'

const props = defineProps<{
  proxy: OutboundProxyRecord | null
  saving: boolean
  testing: boolean
  testResult: OutboundProxyTest | null
}>()
const emit = defineEmits<{
  save: []
  test: []
}>()
const open = defineModel<boolean>({ required: true })
const name = defineModel<string>('name', { required: true })
const proxyUrl = defineModel<string>('proxyUrl', { required: true })
const autoLocation = defineModel<boolean>('autoLocation', { required: true })
const customLocation = defineModel<boolean>('customLocation', { required: true })
const location = defineModel<RequestLocation>('location', { required: true })
const testedConnection = computed(() => props.testResult ?? (proxyUrl.value.trim() ? null : props.proxy?.lastTest))
const detection = computed(() => testedConnection.value?.location)
const detectedLocation = computed(() => detection.value?.status === 'detected'
  ? detection.value.location
  : (!proxyUrl.value.trim() ? props.proxy?.detectedLocation?.location : null))
const detectionWarning = computed(() => locationDetectionWarning(testedConnection.value))
const connectionFailure = computed(() => testedConnection.value?.success === false ? testedConnection.value.message : '')
const showSecret = shallowRef(false)
const busy = computed(() => props.saving || props.testing)
const title = computed(() => props.proxy ? '编辑代理' : '新增代理')
const connectionDescription = computed(() => props.proxy
  ? '留空保留当前连接和认证信息，填写新地址时，请包含所需的用户名和密码'
  : '支持 HTTP、HTTPS、SOCKS5 和 SOCKS5H，可在地址中包含用户名和密码')

watch(open, () => {
  showSecret.value = false
})
</script>

<template>
  <BaseModal v-model="open" :title="title" size="md" :dismissible="!busy">
    <BaseForm class="grid gap-5">
      <BaseFormItem label="代理名称" required>
        <BaseInput v-model="name" maxlength="100" :disabled="busy" aria-label="代理名称" placeholder="请输入代理名称" />
      </BaseFormItem>
      <BaseFormItem label="代理地址" :required="!proxy" :description="connectionDescription">
        <BaseInput
          v-model="proxyUrl"
          :type="showSecret ? 'text' : 'password'"
          autocomplete="new-password"
          :disabled="busy"
          aria-label="代理地址"
          placeholder="请输入代理地址"
        >
          <template #suffix>
            <BaseIconButton :label="showSecret ? '隐藏代理地址' : '显示代理地址'" :disabled="busy" @click="showSecret = !showSecret">
              <EyeOff v-if="showSecret" class="size-4" />
              <Eye v-else class="size-4" />
            </BaseIconButton>
          </template>
        </BaseInput>
      </BaseFormItem>
      <div class="grid gap-2">
        <BaseSwitch v-model="autoLocation" label="自动跟随出口 IP 时区" show-label :disabled="busy" />
        <template v-if="autoLocation">
          <p class="m-0 text-cp-xs text-cp-text-secondary">
            首次开启或更换地址时识别，保存后仅在测试连接时刷新
          </p>
          <p v-if="detectedLocation" class="m-0 break-words text-cp-sm text-cp-text">
            {{ detectedLocation.country }} / {{ detectedLocation.region }} / {{ detectedLocation.city }} · {{ detectedLocation.timezone }}
          </p>
          <p v-if="connectionFailure" class="m-0 text-cp-sm text-cp-error-text" role="alert">
            连接测试失败：{{ connectionFailure }}
          </p>
          <p v-if="detectionWarning && !connectionFailure" class="m-0 text-cp-sm text-cp-warning-text" role="status">
            {{ detectionWarning }}{{ detectedLocation ? '，沿用上次位置' : detection?.status === 'conflict' ? '' : '，未使用自动位置覆盖' }}
          </p>
          <p v-else-if="!detectedLocation && !connectionFailure" class="m-0 text-cp-sm text-cp-text-secondary">
            保存时自动识别出口位置
          </p>
          <p v-if="proxy?.detectedLocation && !proxyUrl.trim() && !testResult" class="m-0 text-cp-xs text-cp-text-tertiary">
            最近识别 {{ formatDateTime(proxy.detectedLocation.detectedAt) }}
          </p>
        </template>
      </div>
      <template v-if="!autoLocation">
        <BaseSwitch v-model="customLocation" label="自定义时区位置" show-label :disabled="busy" />
        <RequestLocationFields v-if="customLocation" v-model="location" :disabled="busy" />
      </template>
      <p v-if="proxy?.accountCount && proxyUrl.trim()" class="m-0 text-cp-sm text-cp-warning-text">
        将更新 {{ proxy.accountCount }} 个关联账号的出口
      </p>
    </BaseForm>
    <template #footer>
      <BaseButton variant="secondary" :disabled="busy" @click="open = false">
        取消
      </BaseButton>
      <BaseButton variant="secondary" :loading="testing" :disabled="saving" @click="emit('test')">
        <template #icon>
          <Wifi class="size-4" />
        </template>
        测试连接
      </BaseButton>
      <BaseButton variant="primary" :loading="saving" :disabled="testing" @click="emit('save')">
        <template #icon>
          <Save class="size-4" />
        </template>
        保存代理
      </BaseButton>
    </template>
  </BaseModal>
</template>
