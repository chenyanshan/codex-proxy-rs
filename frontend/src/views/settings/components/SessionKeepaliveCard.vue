<script setup lang="ts">
import { shallowRef } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseConfirmModal from '@/components/base/BaseConfirmModal.vue'
import BaseSwitch from '@/components/base/BaseSwitch.vue'

const props = defineProps<{ disabled: boolean }>()
const enabled = defineModel<boolean>({ required: true })
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
  <BaseCard title="会话保活">
    <div class="grid gap-3">
      <BaseSwitch :model-value="enabled" label="开启 State 重写" show-label :disabled="disabled" @click.capture.prevent="toggle" />
      <p class="m-0 text-cp-sm text-cp-text-secondary">
        默认关闭。启用后，仅对手动开启的账户和所选模型定期发送 hi，并更新各自的 State。
        请先在 <RouterLink to="/proxies" class="text-cp-primary-text">
          代理管理
        </RouterLink> 保存一个动态代理并测试通过。业务出口保持不变。
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
