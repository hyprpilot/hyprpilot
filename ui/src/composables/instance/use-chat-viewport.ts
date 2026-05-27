/**
 * Surface composable for the chat body.
 *
 * Combines `useInstanceChatInfiniteQuery` (the data layer) with the
 * flattened oldest-first items view + `latestSeq` cursor consumers
 * read. The frontend no longer lazy-loads viewport windows; the
 * initial snapshot asks the daemon for its full retained transcript
 * ring and live patching appends newer items in place.
 *
 * Live patching (`acp:transcript` + `acp:permission-resolved`) lives
 * in the module-level singleton at `./transcript-patcher.ts`. The
 * earlier per-composable IIFE only wired itself when this composable
 * mounted, which on remote landed after the WS auto-subscribed to
 * events — every event arriving in that gap was dropped on the floor
 * at the remote-bridge dispatcher (no listener registered for the
 * event name yet). `startTranscriptPatcher` runs from `main.ts`
 * before the first boot RPC fires, so the dispatcher's
 * `eventListeners.get('acp:transcript')` is populated by the time
 * the daemon pushes its first frame.
 */

import { type InfiniteData } from '@tanstack/vue-query'
import { computed, type ComputedRef } from 'vue'

import { useInstanceChatInfiniteQuery, type UseInstanceChatInfiniteQueryReturn } from './use-instance-chat-infinite-query'
import { type InstanceId } from '../chrome/use-active-instance'
import { type ChatSnapshot, type SeqTranscriptItem } from '@ipc'

export interface UseChatViewportApi {
  /** Flattened, oldest-first transcript items across every cached page. */
  items: ComputedRef<SeqTranscriptItem[]>
  /** Latest known `seq` cursor for reconnect delta replay. */
  latestSeq: ComputedRef<number | undefined>
  /** Initial-load gate; `true` while the retained snapshot is in flight. */
  isInitialLoading: ComputedRef<boolean>
}

interface PatchableInfiniteData extends InfiniteData<ChatSnapshot, number | undefined> {
  pages: ChatSnapshot[]
  pageParams: (number | undefined)[]
}

export function useChatViewport(instanceId: ComputedRef<InstanceId | undefined>): UseChatViewportApi {
  const query: UseInstanceChatInfiniteQueryReturn = useInstanceChatInfiniteQuery(instanceId)

  const localSeq: ComputedRef<number | undefined> = computed(() => {
    const data = query.data.value as PatchableInfiniteData | undefined

    if (!data || data.pages.length === 0) {
      return undefined
    }

    for (const page of data.pages) {
      const items = page.items

      if (items.length === 0) {
        continue
      }
      const last = items[items.length - 1]

      if (last && last.seq > (page.latestSeq ?? -1)) {
        return last.seq
      }

      if (page.latestSeq !== undefined) {
        return page.latestSeq
      }
    }

    return undefined
  })

  // Flattened, oldest-first view. The daemon serves the newest page
  // first (page 0); within a page items are oldest-first. To produce
  // an oldest-first stream we walk pages from last to first, then
  // each page's items in their natural order.
  //
  // Seq dedup as defense in depth: live patching mutates page[0]
  // independently of any snapshot refetch. If the same seq ever lands
  // twice, the projector still renders a single transcript item.
  const items = computed<SeqTranscriptItem[]>(() => {
    const data = query.data.value as PatchableInfiniteData | undefined

    if (!data || data.pages.length === 0) {
      return []
    }
    const out: SeqTranscriptItem[] = []
    const seen = new Set<number>()

    for (let p = data.pages.length - 1; p >= 0; p -= 1) {
      const page = data.pages[p]

      if (!page) {
        continue
      }

      for (const it of page.items) {
        if (seen.has(it.seq)) {
          continue
        }
        seen.add(it.seq)
        out.push(it)
      }
    }

    return out
  })

  const isInitialLoading = computed(() => query.isFetching.value && (query.data.value === undefined || (query.data.value as PatchableInfiniteData).pages.length === 0))

  return {
    items,
    latestSeq: localSeq,
    isInitialLoading
  }
}
