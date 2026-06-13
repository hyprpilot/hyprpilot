import { type InfiniteData, type QueryClient } from '@tanstack/vue-query'

import { type InstanceId } from '../chrome/use-active-instance'
import { type ChatSnapshot } from '@ipc'

const PARTIAL_CHAT_CACHE = '__hyprpilotPartial'

export interface ChatInfiniteData extends InfiniteData<ChatSnapshot, number | undefined> {
  pages: ChatSnapshot[]
  pageParams: (number | undefined)[]
  [PARTIAL_CHAT_CACHE]?: true
}

export function snapshotChatKey(instanceId: InstanceId): ['snapshot-chat', InstanceId] {
  return ['snapshot-chat', instanceId]
}

export function emptyPartialChatData(): ChatInfiniteData {
  return {
    pages: [
      {
        items: [],
        oldestSeq: undefined,
        latestSeq: undefined,
        hasMore: false
      }
    ],
    pageParams: [undefined],
    [PARTIAL_CHAT_CACHE]: true
  }
}

export function fullChatData(snap: ChatSnapshot): ChatInfiniteData {
  return {
    pages: [snap],
    pageParams: [undefined]
  }
}

export function partialChatData(snap: ChatSnapshot): ChatInfiniteData {
  return {
    ...fullChatData(snap),
    [PARTIAL_CHAT_CACHE]: true
  }
}

export function isChatCachePartial(data: unknown): boolean {
  return typeof data === 'object' && data !== null && (data as Record<string, unknown>)[PARTIAL_CHAT_CACHE] === true
}

export function getChatCacheData(queryClient: QueryClient, instanceId: InstanceId): ChatInfiniteData | undefined {
  return queryClient.getQueryData<ChatInfiniteData>(snapshotChatKey(instanceId))
}
