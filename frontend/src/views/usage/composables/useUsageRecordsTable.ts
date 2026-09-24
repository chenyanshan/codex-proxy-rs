import type { Ref } from 'vue'
import type { UsageDisplayRecord } from '../utils/records'
import type { UsageTimeRangeParams } from './useUsageTimeRange'
import { watchDebounced } from '@vueuse/core'

import { computed, onMounted, onScopeDispose, shallowRef, watch } from 'vue'
import {
  getUsageProviders,
  getUsageRecordInsightsDiagnostics,
  getUsageRecordInsightsOverview,
  getUsageRecords,
  getUsageRecordSummary,
} from '@/api'
import { useRequestState } from '@/composables/useRequestState'
import { withMinimumDuration } from '@/utils/async'

interface UseUsageRecordsTableOptions {
  timeRangeParams: Readonly<Ref<UsageTimeRangeParams>>
  latestTimeRangeParams: () => UsageTimeRangeParams
  active: Readonly<Ref<boolean>>
}

type UsageLoadScope = 'all' | 'table'

interface UsageLoadOptions {
  scope?: UsageLoadScope
  background?: boolean
}

export function useUsageRecordsTable(options: UseUsageRecordsTableOptions) {
  const loading = shallowRef(true)
  const analyticsLoading = shallowRef(true)
  const records = shallowRef<UsageDisplayRecord[]>([])
  const summary = shallowRef(emptySummary())
  const insights = shallowRef(emptyInsights())
  const currentPage = shallowRef(1)
  const pageSize = shallowRef(10)
  const totalRecords = shallowRef(0)
  const searchQuery = shallowRef('')
  const search = computed(() => searchQuery.value.trim() || undefined)
  const providerQuery = shallowRef('')
  const providers = shallowRef<string[]>([])
  const providerRequest = useRequestState()
  let tableParams = snapshot()
  const refreshingList = shallowRef(false)
  const diagnosticDimension = shallowRef('model')
  const diagnosticPage = shallowRef(1)
  const diagnosticLoading = shallowRef(false)
  let tableRequestId = 0
  let analyticsRequestId = 0
  let diagnosticRequestId = 0
  let tableController: AbortController | undefined
  let analyticsController: AbortController | undefined
  let diagnosticController: AbortController | undefined
  let disposed = false
  const scopedParams = () => ({
    ...options.timeRangeParams.value,
    ...(providerQuery.value ? { provider: providerQuery.value } : {}),
  })
  const usagePagination = computed(() => ({
    currentPage: currentPage.value,
    pageSize: pageSize.value,
    total: totalRecords.value,
  }))

  function snapshot() {
    return {
      ...options.latestTimeRangeParams(),
      provider: providerQuery.value || undefined,
      search: search.value,
    }
  }

  function resetPagination() {
    currentPage.value = 1
    totalRecords.value = 0
  }

  async function loadUsageRecords(loadOptions: UsageLoadOptions = {}) {
    const { scope = 'all', background = false } = loadOptions
    const globalParams = scopedParams()
    if (scope === 'all') {
      resetPagination()
      diagnosticPage.value = 1
      tableParams = snapshot()
    }

    await Promise.all([
      ...(scope === 'all' ? [loadProviders(globalParams)] : []),
      ...(options.active.value ? [loadUsagePage(background)] : []),
      ...(scope === 'all' ? [loadUsageAnalytics(globalParams, background)] : []),
    ])
  }

  async function loadProviders(range = options.latestTimeRangeParams()) {
    const requestId = providerRequest.start()
    try {
      const result = await getUsageProviders({ startTime: range.startTime, endTime: range.endTime }, { signal: providerRequest.signal, silent: true })
      if (providerRequest.isCurrent(requestId))
        providers.value = result
    }
    catch (cause) {
      if (providerRequest.isCurrent(requestId))
        providers.value = []
      providerRequest.fail(requestId, cause)
    }
    finally {
      providerRequest.finish(requestId)
    }
  }

  async function loadUsagePage(background: boolean) {
    const requestId = ++tableRequestId
    tableController?.abort()
    tableController = new AbortController()
    loading.value = !background
    try {
      const result = await getUsageRecords({
        currentPage: currentPage.value,
        pageSize: pageSize.value,
        ...tableParams,
      }, { signal: tableController.signal })
      if (requestId !== tableRequestId)
        return

      records.value = result.items
      pageSize.value = result.pageSize
      totalRecords.value = result.total
      currentPage.value = result.currentPage
    }
    catch {}
    finally {
      if (requestId === tableRequestId) {
        loading.value = false
      }
    }
  }

  async function loadUsageAnalytics(globalParams: ReturnType<typeof scopedParams>, background: boolean) {
    const requestId = ++analyticsRequestId
    const diagnosticsId = ++diagnosticRequestId
    analyticsController?.abort()
    diagnosticController?.abort()
    diagnosticLoading.value = false
    analyticsController = new AbortController()
    const requestOptions = { signal: analyticsController.signal }
    const dimension = diagnosticDimension.value
    const page = diagnosticPage.value
    analyticsLoading.value = !background
    try {
      const [nextSummary, overview, diagnostics] = await Promise.all([
        getUsageRecordSummary(globalParams, requestOptions),
        getUsageRecordInsightsOverview(globalParams, requestOptions),
        getUsageRecordInsightsDiagnostics({
          ...globalParams,
          dimension,
          ...(dimension === 'keyModel' ? { currentPage: page, pageSize: 20 } : {}),
        }, requestOptions),
      ])
      if (requestId !== analyticsRequestId)
        return

      summary.value = nextSummary
      insights.value = {
        overview,
        diagnostics:
          diagnosticsId === diagnosticRequestId && dimension === diagnosticDimension.value
            ? diagnostics
            : insights.value.diagnostics,
      }
    }
    catch {
      if (requestId === analyticsRequestId && diagnosticsId === diagnosticRequestId)
        diagnosticPage.value = insights.value.diagnostics.dimension === dimension ? insights.value.diagnostics.currentPage : 1
    }
    finally {
      if (requestId === analyticsRequestId) {
        analyticsLoading.value = false
      }
    }
  }

  async function loadDiagnostics() {
    const requestId = ++diagnosticRequestId
    diagnosticController?.abort()
    diagnosticController = new AbortController()
    const dimension = diagnosticDimension.value
    const page = diagnosticPage.value
    const params = scopedParams()
    diagnosticLoading.value = true
    try {
      const diagnostics = await getUsageRecordInsightsDiagnostics({
        ...params,
        dimension,
        ...(dimension === 'keyModel' ? { currentPage: page, pageSize: 20 } : {}),
      }, { signal: diagnosticController.signal })
      if (requestId !== diagnosticRequestId || dimension !== diagnosticDimension.value)
        return
      insights.value = {
        ...insights.value,
        diagnostics,
      }
      diagnosticPage.value = diagnostics.currentPage
    }
    catch {
      if (requestId === diagnosticRequestId)
        diagnosticPage.value = insights.value.diagnostics.dimension === dimension ? insights.value.diagnostics.currentPage : 1
    }
    finally {
      if (requestId === diagnosticRequestId)
        diagnosticLoading.value = false
    }
  }

  function handleDiagnosticPageChange(page: number) {
    if (page < 1 || (page > diagnosticPage.value && !insights.value.diagnostics.hasMore))
      return
    diagnosticPage.value = page
    void loadDiagnostics()
  }

  async function refreshUsageRecords() {
    if (refreshingList.value || loading.value)
      return
    refreshingList.value = true
    try {
      await withMinimumDuration(() => Promise.all([reloadLatestTable(), loadProviders()]))
    }
    finally {
      refreshingList.value = false
    }
  }

  function reloadLatestTable() {
    tableParams = snapshot()
    resetPagination()
    return loadUsageRecords({ scope: 'table' })
  }

  function handlePageChange(nextPage: number) {
    if (tableParams.search !== search.value) {
      void reloadLatestTable()
      return
    }
    currentPage.value = nextPage
    void loadUsageRecords({ scope: 'table' })
  }

  function handlePageSizeChange(nextPageSize: number) {
    pageSize.value = nextPageSize
    if (tableParams.search !== search.value) {
      void reloadLatestTable()
      return
    }
    resetPagination()
    void loadUsageRecords({ scope: 'table' })
  }

  onMounted(() => {
    loadUsageRecords()
  })

  watch(diagnosticDimension, () => {
    diagnosticPage.value = 1
    void loadDiagnostics()
  })

  watch(providerQuery, () => {
    void loadUsageRecords({ background: true })
  })

  watch(options.active, (active) => {
    if (active) {
      void reloadLatestTable()
    }
    else {
      tableRequestId += 1
      tableController?.abort()
    }
  })

  watchDebounced(
    search,
    () => {
      if (!disposed && options.active.value && tableParams.search !== search.value)
        void reloadLatestTable()
    },
    { debounce: 250 },
  )

  onScopeDispose(() => {
    disposed = true
    tableRequestId += 1
    analyticsRequestId += 1
    diagnosticRequestId += 1
    tableController?.abort()
    analyticsController?.abort()
    diagnosticController?.abort()
  })

  return {
    currentPage,
    pageSize,
    searchQuery,
    providerQuery,
    providers,
    providersLoading: providerRequest.loading,
    providersError: providerRequest.error,
    loadProviders,
    usagePagination,
    loading,
    analyticsLoading,
    records,
    summary,
    insights,
    refreshingList,
    diagnosticDimension,
    diagnosticLoading,
    loadUsageRecords,
    refreshUsageRecords,
    handlePageChange,
    handlePageSizeChange,
    handleDiagnosticPageChange,
  }
}

function emptySummary() {
  const summary: Awaited<ReturnType<typeof getUsageRecordSummary>> = {
    totalRequests: '0',
    inputTokens: '0',
    outputTokens: '0',
    cachedTokens: '0',
    cacheWriteTokens: '0',
    totalTokens: '0',
    averageLatencyMs: '0 ms',
  }
  return summary
}

function emptyInsights() {
  return {
    overview: emptyOverview(),
    diagnostics: emptyDiagnostics(),
  }
}

function emptyOverview() {
  const overview: Awaited<ReturnType<typeof getUsageRecordInsightsOverview>> = {
    granularity: '1d',
    health: {
      totalRequests: 0,
      successRequests: 0,
      failedRequests: 0,
      cancelledRequests: 0,
      incompleteRequests: 0,
      callerErrorRequests: 0,
      successRate: 0,
      completionRate: 0,
      requestChangeRate: null,
      successRateChange: null,
      points: [],
    },
    performance: {
      latencyP50Ms: null,
      latencyP95Ms: null,
      latencyP99Ms: null,
      firstTokenP50Ms: null,
      firstTokenP95Ms: null,
      firstTokenP99Ms: null,
      admissionDecisionP50Ms: null,
      admissionDecisionP95Ms: null,
      accountSelectionWaitP50Ms: null,
      accountSelectionWaitP95Ms: null,
      outputThroughputP10: null,
      outputThroughputP50: null,
      outputThroughputP90: null,
      capacityUtilization: null,
      capacityUtilizationP95: null,
      latencyCoverage: 0,
      firstTokenCoverage: 0,
      admissionDecisionCoverage: 0,
      accountSelectionWaitCoverage: 0,
      capacityCoverage: 0,
      points: [],
    },
    cost: {
      estimatedCost: null,
      standardCost: null,
      noCacheCost: null,
      cacheSavings: null,
      tierPremium: null,
      costPerRequest: null,
      costPerSuccessfulRequest: null,
      tokensPerRequest: 0,
      cachedTokenRate: 0,
      cacheHitRequestRate: 0,
      inputTokens: 0,
      outputTokens: 0,
      cachedTokens: 0,
      totalTokens: 0,
      points: [],
      coverage: { known: 0, partial: 0, unknown: 0, notBillable: 0 },
    },
  }
  return overview
}

function emptyDiagnostics() {
  const diagnostics: Awaited<ReturnType<typeof getUsageRecordInsightsDiagnostics>> = {
    dimension: 'model',
    items: [],
    currentPage: 1,
    pageSize: 100,
    hasMore: false,
  }
  return diagnostics
}
