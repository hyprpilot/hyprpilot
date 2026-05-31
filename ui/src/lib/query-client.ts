import type { QueryClient } from '@tanstack/vue-query'

let current: QueryClient | undefined

export function setAppQueryClient(client: QueryClient): void {
  current = client
}

export function appQueryClient(): QueryClient | undefined {
  return current
}
