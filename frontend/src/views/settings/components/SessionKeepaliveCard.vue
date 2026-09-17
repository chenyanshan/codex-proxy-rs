<script setup lang="ts">
import BaseCard from '@/components/base/BaseCard.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseInput from '@/components/base/BaseInput.vue'

defineProps<{ disabled: boolean }>()
const oamProxy = defineModel<string>({ required: true })
</script>

<template>
  <BaseCard title="会话保活">
    <BaseFormItem label="运维探活代理" description="用于开启保活的账号；留空停止探活。业务请求继续使用各自的原有出口。">
      <BaseInput
        v-model="oamProxy"
        type="password"
        autocomplete="off"
        aria-label="运维探活代理"
        placeholder="http://proxy.example:8181"
        :disabled="disabled"
      />
    </BaseFormItem>
  </BaseCard>
</template>
