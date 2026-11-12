import { useLockFn } from 'ahooks'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import type { ConflictEntry } from '@/services/bindings'
import {
  deleteBundle,
  getBundleConflicts,
  getBundleView,
  refreshBundle,
  setBundleMembers,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'
import { revalidateQuery, useQuery } from '@/services/query-client'
import { debugLog } from '@/utils/debug'

import { isBundleUid } from './bundle-constants'
import type { BundleDraftMember } from './bundle-dialog'
import {
  BUNDLE_CONFLICTS_QUERY_KEY,
  BUNDLE_VIEW_QUERY_KEY,
  refreshBundleQueries,
  shouldRetryBundleRefresh,
} from './bundle-queries'

// Auto member updates materialize the bundle after a debounce plus finalize;
// revalidate past that, then retry only if the result is still stale.
const TIMER_REFRESH_DELAY = 2500
const TIMER_REFRESH_RETRY_DELAY = 3500

export interface BundleConfirmState {
  mode: 'create' | 'overwrite'
  members: BundleDraftMember[]
}

interface Options {
  profileItems: IProfileItem[]
  selectedProfiles: ReadonlySet<string>
  completedUpdateRevisions: ReadonlyMap<string, number>
  onLogsChanged: () => void
  /** Page switches to batch mode and preselects these member uids. */
  onEditMembers: (uids: string[]) => void
  /** Page leaves the bundle selection intent after create/update. */
  onSelectionCleared: () => void
}

/**
 * FORK: owns the Profile Bundle page state: backend view/conflicts, freshness
 * revalidation, selection intent, create/update/delete/refresh actions and the
 * dialogs' open state. Keeps the upstream page wiring small.
 */
export function useProfileBundle({
  profileItems,
  selectedProfiles,
  completedUpdateRevisions,
  onLogsChanged,
  onEditMembers,
  onSelectionCleared,
}: Options) {
  const [editMode, setEditMode] = useState(false)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const [confirm, setConfirm] = useState<BundleConfirmState | null>(null)

  const viewQuery = useQuery({
    queryKey: BUNDLE_VIEW_QUERY_KEY,
    queryFn: getBundleView,
  })
  const view = viewQuery.data ?? null
  const conflictsQuery = useQuery({
    queryKey: BUNDLE_CONFLICTS_QUERY_KEY,
    queryFn: getBundleConflicts,
    enabled: Boolean(view),
  })
  const conflicts: ConflictEntry[] = conflictsQuery.data ?? []

  const viewRef = useRef(view)
  viewRef.current = view
  const memberUids = useMemo(
    () => view?.members.map((member) => member.uid).join('|') ?? '',
    [view],
  )

  const refetchViewRef = useRef(viewQuery.refetch)
  refetchViewRef.current = viewQuery.refetch

  const notifiedErrorRef = useRef<unknown>(null)
  const viewError = viewQuery.error
  useEffect(() => {
    if (viewError && notifiedErrorRef.current !== viewError) {
      notifiedErrorRef.current = viewError
      showNotice.error(viewError)
    }
  }, [viewError])

  // Member updates regenerate the bundle shortly after completion; revalidate
  // past the materializer, then retry once only if the bundle is still stale.
  const memberRevisionsRef = useRef<Map<string, number>>(new Map())
  useEffect(() => {
    const currentView = viewRef.current
    if (!currentView) return
    const previous = memberRevisionsRef.current
    const next = new Map<string, number>()
    let memberUpdated = false
    for (const member of currentView.members) {
      const revision = completedUpdateRevisions.get(member.uid) ?? 0
      next.set(member.uid, revision)
      if (previous.has(member.uid) && previous.get(member.uid) !== revision) {
        memberUpdated = true
      }
    }
    memberRevisionsRef.current = next
    if (!memberUpdated) return
    let cancelled = false
    let retry: ReturnType<typeof setTimeout> | undefined
    const first = setTimeout(() => {
      void (async () => {
        try {
          const result = await refetchViewRef.current()
          await revalidateQuery(BUNDLE_CONFLICTS_QUERY_KEY)
          if (!shouldRetryBundleRefresh(cancelled, result.data?.stale)) return
          retry = setTimeout(() => {
            void refreshBundleQueries().catch((retryError) => {
              debugLog(
                '[ProfileBundle] slow-finalize revalidation failed',
                retryError,
              )
            })
          }, TIMER_REFRESH_RETRY_DELAY)
        } catch (error) {
          // Background freshness check; never leave an unhandled rejection.
          debugLog(
            '[ProfileBundle] bundle freshness revalidation failed',
            error,
          )
        }
      })()
    }, TIMER_REFRESH_DELAY)
    return () => {
      cancelled = true
      clearTimeout(first)
      if (retry) clearTimeout(retry)
    }
  }, [memberUids, completedUpdateRevisions])

  const bundleExists = useMemo(
    () => profileItems.some((item) => isBundleUid(item.uid)),
    [profileItems],
  )
  // The generated bundle is not a normal batch target: it has its own confirmed
  // delete flow and can never be a bundle member.
  const selectableProfiles = useMemo(
    () => profileItems.filter((item) => !isBundleUid(item.uid)),
    [profileItems],
  )
  const selectedMembers = useMemo<BundleDraftMember[]>(
    () =>
      profileItems
        .filter(
          (item) =>
            item.uid &&
            !isBundleUid(item.uid) &&
            selectedProfiles.has(item.uid),
        )
        .map((item) => ({ uid: item.uid!, name: item.name || item.uid! })),
    [profileItems, selectedProfiles],
  )
  const isBundleItem = useCallback(
    (uid?: string | null) => isBundleUid(uid),
    [],
  )

  const requestBundleAction = () => {
    if (selectedMembers.length < 2) return
    setConfirm({
      mode: bundleExists ? 'overwrite' : 'create',
      members: selectedMembers,
    })
  }

  const confirmBundleAction = useLockFn(async () => {
    if (!confirm || confirm.members.length < 2) return
    setBusy(true)
    try {
      await mutate(
        () => setBundleMembers(confirm.members.map((member) => member.uid)),
        {
          id: 'set-bundle-members',
          errorNotice: false,
          revalidate: [
            ['getProfiles'],
            BUNDLE_VIEW_QUERY_KEY,
            BUNDLE_CONFLICTS_QUERY_KEY,
          ],
        },
      )
      setConfirm(null)
      setEditMode(false)
      onSelectionCleared()
      showNotice.success(
        confirm.mode === 'overwrite'
          ? 'bundle.page.notices.updated'
          : 'bundle.page.notices.created',
      )
      onLogsChanged()
    } catch (err: any) {
      showNotice.error(err)
    } finally {
      setBusy(false)
    }
  })

  const editMembers = () => {
    setDetailsOpen(false)
    setEditMode(true)
    onEditMembers((viewRef.current?.members ?? []).map((member) => member.uid))
  }

  const endEdit = useCallback(() => {
    setEditMode(false)
  }, [])

  const refreshBundleAction = useLockFn(async () => {
    setBusy(true)
    try {
      const result = await mutate(() => refreshBundle(), {
        id: 'refresh-bundle',
        errorNotice: false,
        revalidate: [
          ['getProfiles'],
          BUNDLE_VIEW_QUERY_KEY,
          BUNDLE_CONFLICTS_QUERY_KEY,
        ],
      })
      if (!result.ok) {
        showNotice.info('bundle.page.notices.refreshBusy')
        return
      }
      const failed = result.value.failed.length
      if (failed === 0) {
        showNotice.success('bundle.page.notices.refreshed')
      } else {
        showNotice.warning('bundle.page.notices.refreshPartial', {
          count: failed,
        })
      }
    } catch (err: any) {
      showNotice.error(err)
    } finally {
      setBusy(false)
    }
  })

  const refresh = () => void refreshBundleAction()

  const deleteBundleAction = useLockFn(async () => {
    setBusy(true)
    try {
      await mutate(() => deleteBundle(), {
        id: 'delete-bundle',
        errorNotice: false,
        revalidate: [
          ['getProfiles'],
          BUNDLE_VIEW_QUERY_KEY,
          BUNDLE_CONFLICTS_QUERY_KEY,
        ],
      })
      setDeleteOpen(false)
      setDetailsOpen(false)
      showNotice.success('bundle.page.notices.deleted')
      onLogsChanged()
    } catch (err: any) {
      showNotice.error(err)
    } finally {
      setBusy(false)
    }
  })

  const confirmDelete = () => void deleteBundleAction()

  return {
    view,
    conflicts,
    busy,
    editMode,
    detailsOpen,
    deleteOpen,
    confirm,
    bundleExists,
    selectableProfiles,
    selectedMembers,
    isBundleItem,
    requestBundleAction,
    confirmBundle: () => void confirmBundleAction(),
    cancelConfirm: () => setConfirm(null),
    editMembers,
    endEdit,
    refresh,
    openDetails: () => setDetailsOpen(true),
    closeDetails: () => setDetailsOpen(false),
    openDeleteConfirm: () => setDeleteOpen(true),
    closeDeleteConfirm: () => setDeleteOpen(false),
    confirmDelete,
  }
}
