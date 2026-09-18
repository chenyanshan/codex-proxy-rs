<script setup lang="ts">
import { shallowRef } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseConfirmModal from '@/components/base/BaseConfirmModal.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseForm from '@/components/base/BaseForm/index.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseSwitch from '@/components/base/BaseSwitch.vue'

const props = defineProps<{ disabled: boolean }>()
const enabled = defineModel<boolean>({ required: true })
const concurrency = defineModel<string>('concurrency', { required: true })
const retryIntervalSeconds = defineModel<string>('retryIntervalSeconds', { required: true })
const confirming = shallowRef(false)
function toggle() {
  if (props.disabled)
    return
  if (!enabled.value)
    confirming.value = true
  else enabled.value = false
}
function confirm() {
  enabled.value = true
  confirming.value = false
}
</script>

<template>
  <BaseCard title="State 重写">
    <div class="grid gap-3">
      <BaseSwitch :model-value="enabled" label="开启 State 重写" show-label :disabled="disabled" @click.capture.prevent="toggle" />
      <p class="m-0 text-cp-sm text-cp-text-secondary">
        默认关闭。启用后，仅对手动开启的账户和所选模型定期发送 hi，并更新各自的 State。
        请先在 <RouterLink to="/proxies" class="text-cp-primary-text">
          代理管理
        </RouterLink> 保存一个动态代理并测试通过。业务出口保持不变。
      </p>
      <BaseForm class="sm:grid-cols-2">
        <BaseFormItem label="探测并发数" description="每个模型从第一轮起同时发送的请求数，1～10">
          <BaseInput v-model="concurrency" aria-label="State 重写探测并发数" type="number" min="1" max="10" step="1" :disabled="disabled" />
        </BaseFormItem>
        <BaseFormItem label="重试间隔（秒）" description="一轮全部失败后的等待时间，1～300 秒；限流时遵守上游等待要求">
          <BaseInput v-model="retryIntervalSeconds" aria-label="State 重写重试间隔（秒）" type="number" min="1" max="300" step="1" :disabled="disabled" />
        </BaseFormItem>
      </BaseForm>
      <p class="m-0 text-cp-sm text-cp-text-secondary">
        保存后用于手动和后台刷新，下一轮重试读取新值。模型成功后立即停止；后台刷新周期仍为 53～55 分钟。
      </p>
      <p class="m-0 text-cp-sm text-cp-warning-text">
        此功能可能导致账户异常或上游限流，并产生模型调用消耗。确认后还需保存设置才会生效。
      </p>
    </div>
  </BaseCard>
  <BaseConfirmModal v-model="confirming" title="确认开启 State 重写" confirm-text="我已了解风险，开启" @confirm="confirm">
    此功能会主动调用所选模型，可能导致账户异常、限流或额外消耗。是否确认承担风险并开启？
  </BaseConfirmModal>
</template>
