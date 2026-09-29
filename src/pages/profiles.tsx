import { arrayMove } from '@dnd-kit/helpers'
import {
  DragDropProvider,
  KeyboardSensor,
  PointerSensor,
  type DragEndEvent,
  type DragOverEvent,
} from '@dnd-kit/react'
import { isSortable } from '@dnd-kit/react/sortable'
import {
  CheckBoxOutlineBlankRounded,
  CheckBoxRounded,
  ClearRounded,
  ContentPasteRounded,
  DeleteRounded,
  IndeterminateCheckBoxRounded,
  LocalFireDepartmentRounded,
  RefreshRounded,
  TextSnippetOutlined,
  WarningAmberRounded,
} from '@mui/icons-material'
import { Box, Button, Divider, Grid, IconButton, Stack } from '@mui/material'
import { TauriEvent } from '@tauri-apps/api/event'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { readTextFile } from '@tauri-apps/plugin-fs'
import { useLockFn } from 'ahooks'
import { throttle } from 'lodash-es'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useLocation } from 'react-router'
import { closeAllConnections } from 'tauri-plugin-mihomo-api'

import {
  BasePage,
  BaseStyledTextField,
  type DialogRef,
} from '@/components/base'
import { ConflictViewer } from '@/components/profile/conflict-viewer'
import { ProfileItem } from '@/components/profile/profile-item'
import { ProfileMore } from '@/components/profile/profile-more'
import {
  ProfileViewer,
  type ProfileViewerRef,
} from '@/components/profile/profile-viewer'
import { ConfigViewer } from '@/components/setting/mods/config-viewer'
import { useListen } from '@/hooks/use-listen'
import { fetchProfilesIntoCache, useProfiles } from '@/hooks/use-profiles'
import {
  clearMergedProfiles,
  createProfile,
  deleteProfile,
  enhanceProfiles,
  getMergeConflicts,
  getRuntimeLogs,
  importProfile,
  reorderProfile,
  setMergedProfiles,
  updateProfile,
} from '@/services/cmds'
import { subscribeVergeEvents } from '@/services/events'
import { errorDetail, showNotice } from '@/services/notice-service'
import { revalidateQuery, useQuery } from '@/services/query-client'
import {
  useLoadingCache,
  useSetLoadingCache,
  useThemeMode,
} from '@/services/states'
import { debugLog } from '@/utils/debug'
import { isValidUrl } from '@/utils/network'

// 与 src-tauri/src/main.rs 的 worker_limit 上限(8)保持一致，避免前后端更新风暴不对齐
const PROFILE_UPDATE_WORKER_LIMIT = 8
const PROFILE_SWITCH_LOADING_DELAY = 400
const profilePointerSensor = PointerSensor.configure({
  activationConstraints: () => undefined,
})

interface ProfileSwitchRequest {
  profile: string
  notifySuccess: boolean
  force: boolean
}

const debugProfileSwitch = (action: string, profile: string, extra?: any) => {
  const timestamp = new Date().toISOString().substring(11, 23)
  debugLog(`[Profile-Debug][${timestamp}] ${action}: ${profile}`, extra || '')
}

const ProfilePage = () => {
  const { t } = useTranslation()
  const location = useLocation()
  const { addListener } = useListen()
  const [url, setUrl] = useState('')
  const [disabled, setDisabled] = useState(false)
  const [profileDndRevision, setProfileDndRevision] = useState(0)
  const [activatings, setActivatings] = useState<string[]>([])
  const [visibleSwitchingProfile, setVisibleSwitchingProfile] = useState<
    string | null
  >(null)
  const [loading, setLoading] = useState(false)
  const [timerUpdateRevisions, setTimerUpdateRevisions] = useState<
    Map<string, number>
  >(() => new Map())
  const [completedUpdateRevisions, setCompletedUpdateRevisions] = useState<
    Map<string, number>
  >(() => new Map())

  const [batchMode, setBatchMode] = useState(false)
  // FORK: merge membership drives card selection. Upstream's `switchTarget` is
  // intentionally absent — every merge member renders as selected, not only the
  // profile that is currently being switched to.
  const [selectedProfiles, setSelectedProfiles] = useState<Set<string>>(
    () => new Set(),
  )
  // FORK: batch-mode checkboxes are a separate selection from merge membership.
  const [batchSelected, setBatchSelected] = useState<Set<string>>(
    () => new Set(),
  )
  // FORK: local merge order. `reorder_profile` reorders items + merged and re-runs
  // the enhance pipeline, so this only covers the window before the refetch lands.
  const [localActiveOrder, setLocalActiveOrder] = useState<string[]>([])
  const [conflictViewerOpen, setConflictViewerOpen] = useState(false)
  const [conflicts, setConflicts] = useState<ConflictEntry[]>([])

  // Profile 切换在前端串行执行；队列中只保留用户最后一次选择。
  const latestSwitchTargetRef = useRef<string | null>(null)
  const queuedSwitchRef = useRef<ProfileSwitchRequest | null>(null)
  const switchRunnerRef = useRef<Promise<void> | null>(null)
  const switchLoadingTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  )
  const currentProfileRef = useRef<string | undefined>(undefined)
  const profilePageMountedRef = useRef(true)
  const { current } = location.state || {}

  const {
    profiles = {},
    patchProfiles,
    mutateProfiles,
    error,
    isStale,
  } = useProfiles()

  useEffect(() => {
    currentProfileRef.current = profiles.current
  }, [profiles])

  useEffect(() => {
    const handleFileDrop = async () => {
      const unlisten = await addListener(
        TauriEvent.DRAG_DROP,
        async (event: any) => {
          const paths = event.payload.paths

          for (const file of paths) {
            if (!file.endsWith('.yaml') && !file.endsWith('.yml')) {
              showNotice.error('profiles.page.feedback.errors.onlyYaml')
              continue
            }
            const item = {
              type: 'local',
              name: file.split(/\/|\\/).pop() ?? 'New Profile',
              desc: '',
              url: '',
              option: {
                with_proxy: false,
                self_proxy: false,
              },
            } as IProfileItem
            const data = await readTextFile(file)
            await createProfile(item, data)
            await mutateProfiles()
          }
          await enhanceProfiles()
        },
      )

      return unlisten
    }

    const unsubscribe = handleFileDrop()

    return () => {
      unsubscribe.then((cleanup) => cleanup())
    }
  }, [addListener, mutateProfiles])

  const onEmergencyRefresh = useLockFn(async () => {
    debugLog('[紧急刷新] 开始强制刷新所有数据')

    try {
      await Promise.all([revalidateQuery(['getRuntimeLogs']), mutateProfiles()])

      await new Promise((resolve) => setTimeout(resolve, 500))
      await onEnhance(false)

      showNotice.success(
        'profiles.page.feedback.notices.forceRefreshCompleted',
        2000,
      )
    } catch (error) {
      console.error('[紧急刷新] 失败:', error)
      showNotice.error(
        'profiles.page.feedback.notices.emergencyRefreshFailed',
        { message: errorDetail(error) },
        4000,
      )
    }
  })

  const { data: chainLogs = {}, refetch: refetchLogs } = useQuery({
    queryKey: ['getRuntimeLogs'],
    queryFn: getRuntimeLogs,
  })
  const refetchLogsRef = useRef(refetchLogs)
  refetchLogsRef.current = refetchLogs
  const mutateLogs = useCallback(() => refetchLogsRef.current(), [])

  const viewerRef = useRef<ProfileViewerRef>(null)
  const configRef = useRef<DialogRef>(null)

  const profileItems = useMemo(() => {
    const items = profiles.items || []

    const type1 = ['local', 'remote']

    return items.filter((i) => i?.type && type1.includes(i.type))
  }, [profiles])

  // FORK: merge membership. `profiles.merged` (hydrated by the effect below) is the
  // backend truth; before that hydration — or with a single active profile — the
  // current profile is the only member, which keeps the zones stable on first paint.
  const mergeMembers = useMemo(() => {
    if (selectedProfiles.size > 0) return selectedProfiles
    return profiles?.current ? new Set([profiles.current]) : new Set<string>()
  }, [profiles, selectedProfiles])

  // FORK: the merge zone renders first, in merge-priority order, then the rest.
  const activeProfiles = useMemo(() => {
    const preserved = localActiveOrder.filter((uid) => mergeMembers.has(uid))
    const preservedSet = new Set(preserved)
    const appended = [...mergeMembers].filter((uid) => !preservedSet.has(uid))
    const itemsByUid = new Map(profileItems.map((item) => [item.uid, item]))

    return [...preserved, ...appended]
      .map((uid) => itemsByUid.get(uid))
      .filter((item): item is IProfileItem => Boolean(item))
  }, [localActiveOrder, mergeMembers, profileItems])

  const inactiveProfiles = useMemo(
    () => profileItems.filter((item) => !mergeMembers.has(item.uid)),
    [mergeMembers, profileItems],
  )

  // FORK: @dnd-kit/react indexes sortables by render order, so drag indices must be
  // resolved against the array the grid actually renders, not against `profiles.items`.
  const renderedProfiles = useMemo(
    () => [...activeProfiles, ...inactiveProfiles],
    [activeProfiles, inactiveProfiles],
  )

  const currentActivatings = () => {
    return [...new Set([profiles.current ?? ''])].filter(Boolean)
  }

  const onImport = async () => {
    if (!url) return
    if (!isValidUrl(url)) {
      showNotice.error('profiles.page.feedback.errors.invalidUrl')
      return
    }
    setLoading(true)

    const handleImportSuccess = async (noticeKey: string) => {
      showNotice.success(noticeKey)
      setUrl('')
      await performRobustRefresh()
    }
    try {
      await importProfile(url)
      await handleImportSuccess('shared.feedback.notifications.importSuccess')
    } catch (initialErr) {
      console.warn('[订阅导入] 首次导入失败:', initialErr)

      const initialDetail = errorDetail(initialErr)
      if (initialDetail.toLowerCase().includes('legacy tls')) {
        showNotice.error(initialErr)
        return
      }

      showNotice.info('profiles.page.feedback.notifications.importRetry')
      try {
        await importProfile(url, {
          with_proxy: false,
          self_proxy: true,
        })
        await handleImportSuccess(
          'shared.feedback.notifications.importWithClashProxy',
        )
      } catch (retryErr) {
        showNotice.error(
          'profiles.page.feedback.notifications.importFail',
          retryErr,
        )
      }
    } finally {
      setDisabled(false)
      setLoading(false)
    }
  }

  // `useProfiles` already retries three times; add only one business-level retry.
  const performRobustRefresh = async () => {
    let retryCount = 0
    const maxRetries = 1
    const baseDelay = 200

    while (retryCount < maxRetries) {
      try {
        debugLog(`[导入刷新] 第${retryCount + 1}次尝试刷新配置数据`)

        await mutateProfiles()

        await new Promise((resolve) =>
          setTimeout(resolve, baseDelay * (retryCount + 1)),
        )

        await onEnhance(false)
        return
      } catch (error) {
        console.error(`[导入刷新] 第${retryCount + 1}次刷新失败:`, error)
        retryCount++
        await new Promise((resolve) =>
          setTimeout(resolve, baseDelay * retryCount),
        )
      }
    }

    console.warn(`[导入刷新] 常规刷新失败，尝试清除缓存重新获取`)
    try {
      await fetchProfilesIntoCache()
      await onEnhance(false)
      showNotice.error(
        'profiles.page.feedback.notifications.importNeedsRefresh',
        3000,
      )
    } catch (finalError) {
      console.error(`[导入刷新] 最终刷新尝试失败:`, finalError)
      showNotice.error(
        'profiles.page.feedback.notifications.importSuccess',
        5000,
      )
    }
  }

  const refreshMergeConflicts = useCallback(() => {
    getMergeConflicts()
      .then((next) => setConflicts(next))
      .catch((error) => console.error('[merge] 获取冲突信息失败:', error))
  }, [])

  const onDragOver = (event: DragOverEvent) => {
    const { source, target } = event.operation
    if (!isSortable(source) || !isSortable(target)) return

    // FORK: the merge zone and the rest are separate drop surfaces. `reorder_profile`
    // only knows "move A next to B", so a card dragged across the boundary would leave
    // the library's optimistic order and the backend out of sync. Refusing the
    // optimistic reorder keeps the DOM and the merge set consistent.
    const sourceInMerge = mergeMembers.has(String(source.id))
    const targetInMerge = mergeMembers.has(String(target.id))
    if (sourceInMerge !== targetInMerge) {
      event.preventDefault()
    }
  }

  const onDragEnd = async (event: DragEndEvent) => {
    const { operation, canceled } = event
    const { source, target } = operation
    if (canceled || !target || !isSortable(source)) return

    const { index: newIndex, initialIndex: oldIndex } = source.sortable
    if (
      oldIndex < 0 ||
      newIndex < 0 ||
      oldIndex >= renderedProfiles.length ||
      newIndex >= renderedProfiles.length ||
      oldIndex === newIndex
    ) {
      return
    }

    const activeUid = String(source.id)
    const overUid = renderedProfiles[newIndex]?.uid
    if (!overUid || activeUid === overUid) return

    const reordersMerge =
      mergeMembers.has(activeUid) && mergeMembers.has(overUid)
    const reordersIdle =
      !mergeMembers.has(activeUid) && !mergeMembers.has(overUid)
    if (!reordersMerge && !reordersIdle) {
      // Defensive: cross-zone drops are refused in `onDragOver`, so this only runs if
      // one slipped through. Remounting drops the optimistic order.
      setProfileDndRevision((revision) => revision + 1)
      return
    }

    const previousOrder = activeProfiles.map((item) => item.uid)
    if (reordersMerge) {
      setLocalActiveOrder(arrayMove(previousOrder, oldIndex, newIndex))
    }

    try {
      await reorderProfile(activeUid, overUid)
      // 后端 reorder 同步重排 merged 数组，但缓存里的 profiles.merged 还是旧顺序。
      // 切 tab 重新挂载时 hydration effect 会用缓存重置 selectedProfiles → 顺序回退.
      await mutateProfiles()
      if (reordersMerge) refreshMergeConflicts()
    } catch (error) {
      if (reordersMerge) setLocalActiveOrder(previousOrder)
      setProfileDndRevision((revision) => revision + 1)
      showNotice.error(error)
    }
  }

  const executeProfileSwitch = useCallback(
    async ({ profile, notifySuccess, force }: ProfileSwitchRequest) => {
      if (!force && currentProfileRef.current === profile) {
        debugProfileSwitch('ALREADY_CURRENT_IGNORED', profile)
        return
      }

      debugProfileSwitch('SWITCH_START', profile)

      try {
        const outcome = await patchProfiles({ current: profile })
        if (outcome.status === 'busy') {
          debugProfileSwitch('SWITCH_BUSY', profile)
          showNotice.info(
            'profiles.page.feedback.notifications.switchBusy',
            2000,
          )
          return
        }

        if (outcome.status === 'valid') {
          currentProfileRef.current = profile
          void mutateLogs().catch(() => {})
          void closeAllConnections().catch(() => {})

          if (
            notifySuccess &&
            latestSwitchTargetRef.current === profile &&
            queuedSwitchRef.current === null
          ) {
            showNotice.success(
              'profiles.page.feedback.notifications.profileSwitched',
              1000,
            )
          }
          debugProfileSwitch('SWITCH_SUCCESS', profile)
        } else {
          debugProfileSwitch('SWITCH_REJECTED', profile, outcome)
        }
      } catch (err: any) {
        console.error(`[Profile] 切换失败:`, err)
        showNotice.error(err, 4000)
      } finally {
        debugProfileSwitch('SWITCH_END', profile)
      }
    },
    [mutateLogs, patchProfiles],
  )

  const runProfileSwitchQueue = useCallback(async () => {
    while (profilePageMountedRef.current && queuedSwitchRef.current) {
      const request = queuedSwitchRef.current
      queuedSwitchRef.current = null
      await executeProfileSwitch(request)
    }
  }, [executeProfileSwitch])

  const activateProfile = useCallback(
    (profile: string, notifySuccess: boolean, force = false) => {
      if (!profilePageMountedRef.current) return Promise.resolve()

      if (
        !force &&
        currentProfileRef.current === profile &&
        switchRunnerRef.current === null
      ) {
        debugProfileSwitch('ALREADY_CURRENT_IGNORED', profile)
        return Promise.resolve()
      }

      if (
        latestSwitchTargetRef.current === profile &&
        switchRunnerRef.current
      ) {
        debugProfileSwitch('DUPLICATE_SWITCH_IGNORED', profile)
        return switchRunnerRef.current
      }

      latestSwitchTargetRef.current = profile
      queuedSwitchRef.current = { profile, notifySuccess, force }
      setVisibleSwitchingProfile(null)
      if (switchLoadingTimerRef.current) {
        window.clearTimeout(switchLoadingTimerRef.current)
      }
      switchLoadingTimerRef.current = window.setTimeout(() => {
        if (
          profilePageMountedRef.current &&
          latestSwitchTargetRef.current === profile
        ) {
          setVisibleSwitchingProfile(profile)
        }
      }, PROFILE_SWITCH_LOADING_DELAY)

      if (switchRunnerRef.current) {
        debugProfileSwitch('SWITCH_QUEUED', profile)
        return switchRunnerRef.current
      }

      const runner = runProfileSwitchQueue().finally(() => {
        if (switchRunnerRef.current === runner) {
          switchRunnerRef.current = null
          latestSwitchTargetRef.current = null
          if (switchLoadingTimerRef.current) {
            window.clearTimeout(switchLoadingTimerRef.current)
            switchLoadingTimerRef.current = null
          }
          if (profilePageMountedRef.current) {
            setVisibleSwitchingProfile(null)
          }
        }
      })
      switchRunnerRef.current = runner
      return runner
    },
    [runProfileSwitchQueue],
  )

  // FORK: clicking a card toggles merge membership. A single member is a plain
  // profile switch (merged list cleared); two or more become a merge group.
  const onToggleProfile = useLockFn(async (uid: string) => {
    const next = new Set(mergeMembers)
    if (next.has(uid)) {
      if (next.size <= 1) return // must keep at least 1 active
      next.delete(uid)
    } else {
      next.add(uid)
    }

    const previous = selectedProfiles
    setSelectedProfiles(next)
    try {
      if (next.size === 1) {
        const [singleUid] = next
        const outcome = await patchProfiles({ current: singleUid })
        if (outcome.status !== 'valid') {
          setSelectedProfiles(previous)
          showNotice.error(
            'profiles.page.feedback.notifications.profileSwitchFailed',
            4000,
          )
          return
        }
        await clearMergedProfiles()
        setConflicts([])
      } else {
        await setMergedProfiles([...next])
        refreshMergeConflicts()
      }
    } catch (err: any) {
      setSelectedProfiles(previous)
      showNotice.error(err)
    }
  })

  // FORK: hydrate the merge zone from the backend whenever `merged` / `current` change.
  // `mergedUidsKey` keeps the effect from re-running on every refetch of an equal list.
  const mergedUidsKey = useMemo(
    () => (profiles?.merged ?? []).join(','),
    [profiles?.merged],
  )
  const currentUid = profiles?.current ?? ''

  // The query cache owns the merge set, so mirroring it into local state from an effect
  // is the point here rather than an accidental synchronous update.
  /* eslint-disable @eslint-react/set-state-in-effect */
  useEffect(() => {
    const uids = mergedUidsKey ? mergedUidsKey.split(',') : []

    if (uids.length >= 2) {
      setSelectedProfiles(new Set(uids))
      refreshMergeConflicts()
    } else if (currentUid) {
      setSelectedProfiles(new Set([currentUid]))
      setConflicts([])
    }
  }, [currentUid, mergedUidsKey, refreshMergeConflicts])
  /* eslint-enable @eslint-react/set-state-in-effect */

  useEffect(() => {
    let cancelled = false
    void (async () => {
      if (current) {
        await mutateProfiles()
        if (cancelled) return
        await activateProfile(current, false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [current, activateProfile, mutateProfiles])

  const onEnhance = useLockFn(async (notifySuccess: boolean) => {
    if (switchRunnerRef.current) {
      debugLog(
        `[Profile] 有profile正在切换中(${latestSwitchTargetRef.current})，跳过enhance操作`,
      )
      return
    }

    const currentProfiles = currentActivatings()
    setActivatings((prev) => [...new Set([...prev, ...currentProfiles])])

    try {
      if (!(await enhanceProfiles())) return
      mutateLogs()
      if (notifySuccess) {
        showNotice.success(
          'profiles.page.feedback.notifications.profileReactivated',
          1000,
        )
      }
    } catch (err: any) {
      showNotice.error(err, 3000)
    } finally {
      setActivatings([])
    }
  })

  const onDelete = useLockFn(async (uid: string) => {
    const current = profiles.current === uid
    try {
      setActivatings([...(current ? currentActivatings() : []), uid])
      await deleteProfile(uid)
      mutateProfiles()
      mutateLogs()
      if (current) {
        await onEnhance(false)
      }
    } catch (err: any) {
      showNotice.error(err)
    } finally {
      setActivatings([])
    }
  })

  const loadingCache = useLoadingCache()
  const setLoadingCache = useSetLoadingCache()
  const setLoadingProfiles = useCallback(
    (uids: string[], loading: boolean) => {
      setLoadingCache((cache) => {
        const next = new Set(cache)
        for (const uid of uids) {
          if (loading) {
            next.add(uid)
          } else {
            next.delete(uid)
          }
        }
        return next
      })
    },
    [setLoadingCache],
  )

  useEffect(
    () =>
      subscribeVergeEvents({
        'profile-update-started': ({ uid }) => {
          if (uid) setLoadingProfiles([uid], true)
        },
        'profile-update-completed': ({ uid }) => {
          if (!uid) return
          setLoadingProfiles([uid], false)
          setCompletedUpdateRevisions((current) => {
            const next = new Map(current)
            next.set(uid, (next.get(uid) ?? 0) + 1)
            return next
          })
          void mutateProfiles()
        },
        'verge://timer-updated': (uid) => {
          setTimerUpdateRevisions((current) => {
            const next = new Map(current)
            next.set(uid, (next.get(uid) ?? 0) + 1)
            return next
          })
        },
      }),
    [mutateProfiles, setLoadingProfiles],
  )

  const runProfileUpdates = useCallback(
    async (uids: string[]) => {
      if (uids.length === 0) return

      const throttleMutate = throttle(mutateProfiles, 2000, {
        trailing: true,
      })
      let cursor = 0

      const updateOne = async (uid: string) => {
        try {
          await updateProfile(uid)
          throttleMutate()
        } catch (err: any) {
          console.error(`更新订阅 ${uid} 失败:`, err)
        }
      }

      const worker = async () => {
        while (cursor < uids.length) {
          const uid = uids[cursor++]
          await updateOne(uid)
        }
      }

      try {
        const active = Math.min(PROFILE_UPDATE_WORKER_LIMIT, uids.length)
        await Promise.allSettled(Array.from({ length: active }, worker))
      } finally {
        setLoadingProfiles(uids, false)
        void mutateProfiles()
      }
    },
    [mutateProfiles, setLoadingProfiles],
  )
  const onUpdateAll = useLockFn(async () => {
    const items = profileItems.filter((e) => e.type === 'remote')
    const target = items
      .map((item) => item.uid)
      .filter((uid) => !loadingCache.has(uid))

    setLoadingProfiles(target, true)
    await runProfileUpdates(target)
  })

  const onCopyLink = async () => {
    const text = await readText()
    if (text) setUrl(text)
  }

  const toggleBatchMode = () => {
    setBatchMode(!batchMode)
    if (!batchMode) {
      setBatchSelected(new Set())
    }
  }

  const toggleProfileSelection = (uid: string) => {
    setBatchSelected((prev) => {
      const newSet = new Set(prev)
      if (newSet.has(uid)) {
        newSet.delete(uid)
      } else {
        newSet.add(uid)
      }
      return newSet
    })
  }

  const selectAllProfiles = () => {
    setBatchSelected(new Set(profileItems.map((item) => item.uid)))
  }

  const clearAllSelections = () => {
    setBatchSelected(new Set())
  }

  const isAllSelected = () => {
    return profileItems.length > 0 && profileItems.length === batchSelected.size
  }

  const getSelectionState = () => {
    if (batchSelected.size === 0) {
      return 'none'
    } else if (batchSelected.size === profileItems.length) {
      return 'all'
    } else {
      return 'partial'
    }
  }

  const deleteSelectedProfiles = useLockFn(async () => {
    if (batchSelected.size === 0) return

    try {
      const currentActivating =
        profiles.current && batchSelected.has(profiles.current)
          ? [profiles.current]
          : []

      setActivatings((prev) => [...new Set([...prev, ...currentActivating])])

      for (const uid of batchSelected) {
        await deleteProfile(uid)
      }

      await mutateProfiles()
      await mutateLogs()

      if (currentActivating.length > 0) {
        await onEnhance(false)
      }

      setBatchSelected(new Set())
      setBatchMode(false)

      showNotice.success('profiles.page.feedback.notifications.batchDeleted')
    } catch (err: any) {
      showNotice.error(err)
    } finally {
      setActivatings([])
    }
  })

  const mode = useThemeMode()
  const isLight = mode === 'light'
  const dividercolor = isLight
    ? 'rgba(0, 0, 0, 0.06)'
    : 'rgba(255, 255, 255, 0.06)'

  // 卸载后不再执行尚未发送的切换意图。
  useEffect(() => {
    profilePageMountedRef.current = true
    return () => {
      profilePageMountedRef.current = false
      queuedSwitchRef.current = null
      latestSwitchTargetRef.current = null
      if (switchLoadingTimerRef.current) {
        window.clearTimeout(switchLoadingTimerRef.current)
        switchLoadingTimerRef.current = null
      }
    }
  }, [])

  // FORK: cards are rendered from two zones but keep one flat index space, matching
  // the order of `renderedProfiles` that the drag handler resolves indices against.
  const renderProfileCard = (
    item: IProfileItem,
    index: number,
    isMergeMember: boolean,
  ) => {
    const isPrimaryMergeProfile =
      isMergeMember && activeProfiles.length >= 2 && index === 0

    return (
      <Box
        key={item.uid}
        sx={(theme) => ({
          width: '100%',
          minWidth: 0,
          boxSizing: 'border-box',
          // FORK: selected ProfileBox cards shift 3px left; keep them inside the grid column.
          pl: isMergeMember ? '3px' : 0,
          ...(isPrimaryMergeProfile && {
            '& [aria-selected="true"]': {
              borderLeftColor: theme.palette.warning.main,
              '& h2': { color: theme.palette.warning.main },
            },
          }),
        })}
      >
        <ProfileItem
          id={item.uid}
          index={index}
          selected={isMergeMember}
          activating={
            activatings.includes(item.uid) ||
            visibleSwitchingProfile === item.uid
          }
          itemData={item}
          timerUpdateRevision={timerUpdateRevisions.get(item.uid) ?? 0}
          completedUpdateRevision={completedUpdateRevisions.get(item.uid) ?? 0}
          mutateProfiles={mutateProfiles}
          onSelect={() => onToggleProfile(item.uid)}
          onEdit={() => viewerRef.current?.edit(item)}
          onSave={async (prev, curr) => {
            if (prev !== curr && profiles.current === item.uid) {
              await onEnhance(false)
            }
          }}
          onDelete={() => {
            if (batchMode) {
              toggleProfileSelection(item.uid)
            } else {
              onDelete(item.uid)
            }
          }}
          batchMode={batchMode}
          isSelected={batchSelected.has(item.uid)}
          onSelectionChange={() => toggleProfileSelection(item.uid)}
        />
      </Box>
    )
  }

  return (
    <BasePage
      full
      title={t('profiles.page.title')}
      contentStyle={{ height: '100%' }}
      header={
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 1 }}>
          {!batchMode ? (
            <>
              {/* Batch mode toggle button */}
              <IconButton
                size="small"
                color="inherit"
                title={t('profiles.page.batch.title')}
                onClick={toggleBatchMode}
              >
                <CheckBoxOutlineBlankRounded />
              </IconButton>

              {conflicts.length > 0 && (
                <IconButton
                  size="small"
                  color="warning"
                  title={t('profiles.merge.conflicts.badge')}
                  aria-label={`${t('profiles.merge.conflicts.badge')} (${conflicts.length})`}
                  onClick={() => setConflictViewerOpen(true)}
                >
                  <WarningAmberRounded />
                </IconButton>
              )}

              <IconButton
                size="small"
                color="inherit"
                title={t('profiles.page.actions.updateAll')}
                onClick={onUpdateAll}
              >
                <RefreshRounded />
              </IconButton>

              <IconButton
                size="small"
                color="inherit"
                title={t('profiles.page.actions.viewRuntimeConfig')}
                onClick={() => configRef.current?.open()}
              >
                <TextSnippetOutlined />
              </IconButton>

              <IconButton
                size="small"
                color="primary"
                title={t('profiles.page.actions.reactivate')}
                onClick={() => onEnhance(true)}
              >
                <LocalFireDepartmentRounded />
              </IconButton>

              {/* 故障检测和紧急恢复按钮 */}
              {(error || isStale) && (
                <IconButton
                  size="small"
                  color="warning"
                  title={t(
                    'profiles.page.feedback.tooltips.forceRefreshStaleData',
                  )}
                  onClick={onEmergencyRefresh}
                  sx={{
                    animation: 'pulse 2s infinite',
                    '@keyframes pulse': {
                      '0%': { opacity: 1 },
                      '50%': { opacity: 0.5 },
                      '100%': { opacity: 1 },
                    },
                  }}
                >
                  <ClearRounded />
                </IconButton>
              )}
            </>
          ) : (
            <Box sx={{ display: 'flex', alignItems: 'center', gap: 1 }}>
              <IconButton
                size="small"
                color="inherit"
                title={
                  isAllSelected()
                    ? t('profiles.page.batch.actions.deselectAll')
                    : t('profiles.page.batch.actions.selectAll')
                }
                onClick={
                  isAllSelected() ? clearAllSelections : selectAllProfiles
                }
              >
                {getSelectionState() === 'all' ? (
                  <CheckBoxRounded />
                ) : getSelectionState() === 'partial' ? (
                  <IndeterminateCheckBoxRounded />
                ) : (
                  <CheckBoxOutlineBlankRounded />
                )}
              </IconButton>
              <IconButton
                size="small"
                color="error"
                title={t('profiles.page.batch.actions.delete')}
                onClick={deleteSelectedProfiles}
                disabled={batchSelected.size === 0}
              >
                <DeleteRounded />
              </IconButton>
              <Button size="small" variant="outlined" onClick={toggleBatchMode}>
                {t('profiles.page.batch.actions.done')}
              </Button>
              <Box
                sx={{ flex: 1, textAlign: 'right', color: 'text.secondary' }}
              >
                {t('profiles.page.batch.summary.selected')} {batchSelected.size}{' '}
                {t('profiles.page.batch.summary.items')}
              </Box>
            </Box>
          )}
        </Box>
      }
    >
      <Stack
        direction="row"
        spacing={1}
        sx={{
          pt: 1,
          mb: 0.5,
          mx: '10px',
          height: '36px',
          display: 'flex',
          alignItems: 'center',
        }}
      >
        <BaseStyledTextField
          value={url}
          variant="outlined"
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.nativeEvent.isComposing) {
              return
            }
            if (!url || disabled || loading) {
              return
            }
            event.preventDefault()
            void onImport()
          }}
          placeholder={t('profiles.page.importForm.placeholder')}
          slotProps={{
            input: {
              sx: { pr: 1 },
              endAdornment: !url ? (
                <IconButton
                  size="small"
                  sx={{ p: 0.5 }}
                  title={t('profiles.page.importForm.actions.paste')}
                  onClick={onCopyLink}
                >
                  <ContentPasteRounded fontSize="inherit" />
                </IconButton>
              ) : (
                <IconButton
                  size="small"
                  sx={{ p: 0.5 }}
                  title={t('shared.actions.clear')}
                  onClick={() => setUrl('')}
                >
                  <ClearRounded fontSize="inherit" />
                </IconButton>
              ),
            },
          }}
        />
        <Button
          disabled={!url || disabled}
          loading={loading}
          variant="contained"
          size="small"
          sx={{ borderRadius: '6px' }}
          onClick={onImport}
        >
          {t('profiles.page.actions.import')}
        </Button>
        <Button
          variant="contained"
          size="small"
          sx={{ borderRadius: '6px' }}
          onClick={() => viewerRef.current?.create()}
        >
          {t('shared.actions.new')}
        </Button>
      </Stack>

      <Box
        sx={{
          pl: '10px',
          pr: '10px',
          height: 'calc(100% - 48px)',
          overflowY: 'auto',
        }}
      >
        <DragDropProvider
          key={profileDndRevision}
          sensors={[profilePointerSensor, KeyboardSensor]}
          onDragOver={onDragOver}
          onDragEnd={onDragEnd}
        >
          <Box
            sx={{
              mb: 1.5,
              display: 'grid',
              overflow: 'hidden',
              gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))',
              gap: 1,
              px: 0.5,
            }}
          >
            {/* FORK: merge zone — drag to reorder merge priority */}
            {activeProfiles.map((item, index) =>
              renderProfileCard(item, index, true),
            )}
            {activeProfiles.length > 0 && inactiveProfiles.length > 0 && (
              <Divider
                key="fork-merge-zone-divider"
                sx={{
                  gridColumn: '1 / -1',
                  my: 0.5,
                  borderColor: dividercolor,
                }}
              />
            )}
            {/* FORK: everything outside the merge group */}
            {inactiveProfiles.map((item, index) =>
              renderProfileCard(item, activeProfiles.length + index, false),
            )}
          </Box>
        </DragDropProvider>
        <Divider
          variant="middle"
          flexItem
          sx={{ width: `calc(100% - 32px)`, borderColor: dividercolor }}
        ></Divider>
        <Box sx={{ mt: 1.5, mb: '10px' }}>
          <Grid container spacing={{ xs: 1, lg: 1 }}>
            <Grid size={{ xs: 12, sm: 6, md: 6, lg: 6 }}>
              <ProfileMore
                id="Merge"
                onSave={async (prev, curr) => {
                  if (prev !== curr) {
                    await onEnhance(false)
                  }
                }}
              />
            </Grid>
            <Grid size={{ xs: 12, sm: 6, md: 6, lg: 6 }}>
              <ProfileMore
                id="Script"
                logInfo={chainLogs['Script']}
                onSave={async (prev, curr) => {
                  if (prev !== curr) {
                    await onEnhance(false)
                  }
                }}
              />
            </Grid>
          </Grid>
        </Box>
      </Box>

      <ProfileViewer
        ref={viewerRef}
        onChange={async (isActivating) => {
          mutateProfiles()
          if (isActivating) {
            await onEnhance(false)
          }
        }}
      />
      <ConfigViewer ref={configRef} />
      {/* FORK: conflict viewer dialog for multi-profile merge */}
      <ConflictViewer
        open={conflictViewerOpen}
        conflicts={conflicts}
        onClose={() => setConflictViewerOpen(false)}
      />
    </BasePage>
  )
}

export default ProfilePage
