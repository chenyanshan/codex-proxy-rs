import request from '../request'

export interface ClientKeyUsage {
  object: 'key_usage'
  asOf: string
  currency: 'USD'
  timezone: 'Asia/Shanghai'
  dailyLimitUsd: string
  dailyUsedUsd: string
  dailyRemainingUsd: string | null
  dailyResetsAt: string | null
  weeklyLimitUsd: string
  weeklyUsedUsd: string
  weeklyRemainingUsd: string | null
  weeklyResetsAt: string | null
  maxConcurrency: number
  requestsPerMinute: number
}

export function getClientKeyUsage(key: string, signal?: AbortSignal) {
  return request<ClientKeyUsage>({
    url: '/v1/usage',
    method: 'GET',
    headers: { Authorization: `Bearer ${key}` },
    authentication: 'client-key',
    silent: true,
    signal,
  })
}
