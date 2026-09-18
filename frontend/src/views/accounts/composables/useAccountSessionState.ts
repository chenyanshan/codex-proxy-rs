import type { AccountRow } from '../constants'
import type { SessionStateRefresh } from '@/api'
import { shallowRef, watch } from 'vue'
import { streamAccountSessionState } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

export function useAccountSessionState() {
  const account = shallowRef<AccountRow | null>(null)
  const open = shallowRef(false)
  const result = shallowRef<SessionStateRefresh | null>(null)
  const request = useRequestState()

  async function refresh(selected: AccountRow) {
    if (request.loading.value || !selected.enabled || !selected.enableSessionKeepalive)
      return
    account.value = selected
    result.value = null
    open.value = true
    const id = request.start()
    try {
      const data = await streamAccountSessionState({ accountId: selected.id }, (model) => {
        if (!request.isCurrent(id))
          return
        const models = result.value?.models ?? []
        result.value = {
          accountId: selected.id,
          models: [...models.filter(item => item.model !== model.model), model],
        }
      }, { signal: request.signal })
      if (request.isCurrent(id))
        result.value = data
    }
    catch (error) {
      request.fail(id, error)
    }
    finally {
      request.finish(id)
    }
  }

  function cancel() {
    request.invalidate()
  }

  watch(open, (visible) => {
    if (!visible)
      cancel()
  })

  return { account, open, result, loading: request.loading, error: request.error, refresh, cancel }
}
