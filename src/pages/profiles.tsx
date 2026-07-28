import {
  closestCenter,
  DndContext,
  type DragEndEvent,
  type DragStartEvent,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  type SortingStrategy,
} from '@dnd-kit/sortable'
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
import { listen, TauriEvent } from '@tauri-apps/api/event'
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
import { ProfileMore } from '@/components/profile/profile-more'
import {
  ProfileViewer,
  type ProfileViewerRef,
} from '@/components/profile/profile-viewer'
import { SortableProfileItem } from '@/components/profile/sortable-profile-item'
import { ConfigViewer } from '@/components/setting/mods/config-viewer'
import { useListen } from '@/hooks/use-listen'
import { useProfiles } from '@/hooks/use-profiles'
import {
  clearMergedProfiles,
  createProfile,
  deleteProfile,
  enhanceProfiles,
  getMergeConflicts,
  getProfiles,
  //restartCore,
  getRuntimeLogs,
  importProfile,
  reorderProfile,
  setMergedProfiles,
  updateProfile,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import {
  fetchCacheData,
  revalidateQueries,
  useQuery,
} from '@/services/query-client'
import {
  useLoadingCache,
  useSetLoadingCache,
  useThemeMode,
} from '@/services/states'
import { debugLog } from '@/utils/debug'

// 与 src-tauri/src/main.rs 的 worker_limit 上限(8)保持一致，避免前后端更新风暴不对齐
const PROFILE_UPDATE_WORKER_LIMIT = 8
const PROFILE_SWITCH_LOADING_DELAY = 400

// Equivalent to rectSortingStrategy without copying the full rect array for every item.
const profileRectSortingStrategy: SortingStrategy = ({
  rects,
  activeIndex,
  overIndex,
  index,
}) => {
  let newIndex = index

  if (index === activeIndex) {
    newIndex = overIndex
  } else if (
    activeIndex < overIndex &&
    index > activeIndex &&
    index <= overIndex
  ) {
    newIndex = index - 1
  } else if (
    activeIndex > overIndex &&
    index >= overIndex &&
    index < activeIndex
  ) {
    newIndex = index + 1
  }

  const oldRect = rects[index]
  const newRect = rects[newIndex]
  if (!oldRect || !newRect) return null

  return {
    x: newRect.left - oldRect.left,
    y: newRect.top - oldRect.top,
    scaleX: newRect.width / oldRect.width,
    scaleY: newRect.height / oldRect.height,
  }
}

interface ProfileSwitchRequest {
  profile: string
  notifySuccess: boolean
  force: boolean
}

// 记录profile切换状态
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

  const [draggingId, setDraggingId] = useState<string | null>(null)
  // FORK: Local drag order — decoupled from SWR to prevent revalidation snap-back
  const [localActiveOrder, setLocalActiveOrder] = useState<string[]>([])

  // FORK: Multi-profile merge state — selectedProfiles always mirrors active state
  const [selectedProfiles, setSelectedProfiles] = useState<Set<string>>(
    () => new Set(),
  )
  // Batch selection states
  const [batchMode, setBatchMode] = useState(false)
  const [batchSelected, setBatchSelected] = useState<Set<string>>(
    () => new Set(),
  )
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
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  )
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

  // 添加紧急恢复功能
  const onEmergencyRefresh = useLockFn(async () => {
    debugLog('[紧急刷新] 开始强制刷新所有数据')

    try {
      // 只失效 profiles 相关 query，不影响 WS 订阅、IP 缓存等其他 query
      await revalidateQueries([['getProfiles'], ['getRuntimeLogs']])

      // 强制重新获取配置数据
      await mutateProfiles()

      // 等待状态稳定后增强配置
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
        { message: String(error) },
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

  // distinguish type
  const profileItems = useMemo(() => {
    const items = profiles.items || []

    const type1 = ['local', 'remote']

    return items.filter((i) => i && type1.includes(i.type!))
  }, [profiles])

  const activeProfiles = useMemo(() => {
    // During drag: use localActiveOrder for snap-back-free reordering.
    // Otherwise: derive from selectedProfiles, preserving any prior local order.
    const baseOrder = draggingId
      ? localActiveOrder.filter((uid) => selectedProfiles.has(uid))
      : (() => {
          const preserved = localActiveOrder.filter((uid) =>
            selectedProfiles.has(uid),
          )
          const added = [...selectedProfiles].filter(
            (uid) => !localActiveOrder.includes(uid),
          )
          return [...preserved, ...added]
        })()
    return baseOrder
      .map((uid) => profileItems.find((p) => p.uid === uid))
      .filter((p): p is IProfileItem => Boolean(p))
  }, [profileItems, localActiveOrder, selectedProfiles, draggingId])

  const inactiveProfiles = useMemo(
    () => profileItems.filter((p) => !selectedProfiles.has(p.uid!)),
    [profileItems, selectedProfiles],
  )

  const currentActivatings = () => {
    return [...new Set([profiles.current ?? ''])].filter(Boolean)
  }

  const onImport = async () => {
    if (!url) return
    // 校验url是否为http/https
    if (!/^https?:\/\//i.test(url)) {
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
      // 尝试正常导入
      await importProfile(url)
      await handleImportSuccess('shared.feedback.notifications.importSuccess')
    } catch (initialErr) {
      console.warn('[订阅导入] 首次导入失败:', initialErr)

      if (String(initialErr).toLowerCase().includes('legacy tls')) {
        showNotice.error(String(initialErr))
        return
      }

      showNotice.info('profiles.page.feedback.notifications.importRetry')
      try {
        // 使用自身代理尝试导入
        await importProfile(url, {
          with_proxy: false,
          self_proxy: true,
        })
        await handleImportSuccess(
          'shared.feedback.notifications.importWithClashProxy',
        )
      } catch (retryErr) {
        // 回退导入也失败
        showNotice.error(
          'profiles.page.feedback.notifications.importFail',
          String(retryErr),
        )
      }
    } finally {
      setDisabled(false)
      setLoading(false)
    }
  }

  // 强化的刷新策略
  // maxRetries 设为 1：useProfiles 内部 useQuery 已配置 retry:3，业务层只需 1 次额外重试
  const performRobustRefresh = async () => {
    let retryCount = 0
    const maxRetries = 1
    const baseDelay = 200

    while (retryCount < maxRetries) {
      try {
        debugLog(`[导入刷新] 第${retryCount + 1}次尝试刷新配置数据`)

        // 强制刷新，绕过所有缓存
        await mutateProfiles()

        // 等待状态稳定
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

    // 所有重试失败后的最后尝试
    console.warn(`[导入刷新] 常规刷新失败，尝试清除缓存重新获取`)
    try {
      // 清除缓存并重新获取
      await fetchCacheData(['getProfiles'], getProfiles)
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

  const onDragStart = (event: DragStartEvent) => {
    const id = event.active.id.toString()
    // Seed localActiveOrder on first drag so active zone never collapses to empty.
    setLocalActiveOrder((prev) =>
      prev.length > 0 ? prev : activeProfiles.map((p) => p.uid!),
    )
    setDraggingId(id)
  }

  const onDragEnd = async (event: DragEndEvent) => {
    setDraggingId(null)
    const { active, over } = event
    if (!over || active.id === over.id) return

    const activeUid = active.id.toString()
    const overUid = over.id.toString()
    const oldOrder =
      localActiveOrder.length > 0
        ? localActiveOrder
        : activeProfiles.map((p) => p.uid!)
    const oldIdx = oldOrder.indexOf(activeUid)
    const newIdx = oldOrder.indexOf(overUid)
    if (oldIdx === -1 || newIdx === -1) return

    const newOrder = arrayMove(oldOrder, oldIdx, newIdx)
    setLocalActiveOrder(newOrder)

    try {
      await reorderProfile(activeUid, overUid)
      await mutateProfiles()
      const c = await getMergeConflicts()
      setConflicts(c)
    } catch {
      setLocalActiveOrder(oldOrder)
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

  const onToggleProfile = useLockFn(async (uid: string) => {
    const newSet = new Set(selectedProfiles)
    if (newSet.has(uid)) {
      if (newSet.size <= 1) return // must keep at least 1 active
      newSet.delete(uid)
    } else {
      newSet.add(uid)
    }

    const prevSet = selectedProfiles
    setSelectedProfiles(newSet)
    try {
      if (newSet.size === 1) {
        const singleUid = [...newSet][0]
        const outcome = await patchProfiles({ current: singleUid })
        if (outcome.status !== 'valid') {
          setSelectedProfiles(prevSet)
          showNotice.error(
            'profiles.page.feedback.notifications.profileSwitchFailed',
            4000,
          )
          return
        }
        await clearMergedProfiles()
        setConflicts([])
      } else {
        await setMergedProfiles([...newSet])
        const c = await getMergeConflicts()
        setConflicts(c)
      }
    } catch (err: any) {
      setSelectedProfiles(prevSet)
      showNotice.error(err)
    }
  })

  const mergedUidsKey = useMemo(
    () => (profiles?.merged ?? []).join(','),
    [profiles?.merged],
  )
  const currentUid = profiles?.current ?? ''
  useEffect(() => {
    let cancelled = false
    const uids = mergedUidsKey ? mergedUidsKey.split(',') : []
    void Promise.resolve().then(() => {
      if (cancelled) return
      if (uids.length >= 2) {
        setSelectedProfiles(new Set(uids))
        getMergeConflicts()
          .then((nextConflicts) => {
            if (!cancelled) setConflicts(nextConflicts)
          })
          .catch((e) => console.error('[merge] failed to load conflicts', e))
      } else if (currentUid) {
        setSelectedProfiles(new Set([currentUid]))
        setConflicts([])
      }
    })
    return () => {
      cancelled = true
    }
  }, [mergedUidsKey, currentUid])

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

  // 更新所有订阅
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

  useEffect(() => {
    let disposed = false
    let unlisteners: Array<() => void> = []

    Promise.allSettled([
      listen<{ uid?: string }>('profile-update-started', ({ payload }) => {
        if (payload.uid) setLoadingProfiles([payload.uid], true)
      }),
      listen<{ uid?: string }>('profile-update-completed', ({ payload }) => {
        const { uid } = payload
        if (!uid) return
        setLoadingProfiles([uid], false)
        setCompletedUpdateRevisions((current) => {
          const next = new Map(current)
          next.set(uid, (next.get(uid) ?? 0) + 1)
          return next
        })
        void mutateProfiles()
      }),
      listen<string>('verge://timer-updated', ({ payload: uid }) => {
        setTimerUpdateRevisions((current) => {
          const next = new Map(current)
          next.set(uid, (next.get(uid) ?? 0) + 1)
          return next
        })
      }),
    ]).then((results) => {
      const registeredUnlisteners = results.flatMap((result) =>
        result.status === 'fulfilled' ? [result.value] : [],
      )
      results.forEach((result) => {
        if (result.status === 'rejected') console.error(result.reason)
      })

      if (disposed) {
        registeredUnlisteners.forEach((unlisten) => unlisten())
      } else {
        unlisteners = registeredUnlisteners
      }
    })

    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => unlisten())
    }
  }, [mutateProfiles, setLoadingProfiles])

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
        // 避免长时间批量更新后列表数据过晚刷新
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

  // Batch selection functions
  const toggleBatchMode = () => {
    setBatchMode(!batchMode)
    if (!batchMode) {
      // Entering batch mode - clear previous selections
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
      return 'none' // 无选择
    } else if (batchSelected.size === profileItems.length) {
      return 'all' // 全选
    } else {
      return 'partial' // 部分选择
    }
  }

  const deleteSelectedProfiles = useLockFn(async () => {
    if (batchSelected.size === 0) return

    try {
      // Get all currently activating profiles
      const currentActivating =
        profiles.current && batchSelected.has(profiles.current)
          ? [profiles.current]
          : []

      setActivatings((prev) => [...new Set([...prev, ...currentActivating])])

      // Delete all selected profiles
      for (const uid of batchSelected) {
        await deleteProfile(uid)
      }

      await mutateProfiles()
      await mutateLogs()

      // If any deleted profile was current, enhance profiles
      if (currentActivating.length > 0) {
        await onEnhance(false)
      }

      // Clear selections and exit batch mode
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
                  title="数据异常，点击强制刷新"
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
            // Batch mode header
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

      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
      >
        <Box
          sx={{
            pl: '10px',
            pr: '10px',
            height: 'calc(100% - 48px)',
            overflowY: 'auto',
          }}
        >
          {/* Active zone — drag to reorder merge priority */}
          <Box sx={{ mb: 1.5 }}>
            {conflicts.length > 0 && (
              <Box sx={{ display: 'flex', justifyContent: 'flex-end', mb: 1 }}>
                <Button
                  color="warning"
                  size="small"
                  startIcon={<WarningAmberRounded />}
                  variant="outlined"
                  onClick={() => setConflictViewerOpen(true)}
                >
                  {t('profiles.merge.conflicts.badge')} ({conflicts.length})
                </Button>
              </Box>
            )}
            <Grid container spacing={{ xs: 1, lg: 1 }}>
              <SortableContext
                strategy={profileRectSortingStrategy}
                items={activeProfiles.map((p) => p.uid!)}
              >
                {activeProfiles.map((item) => {
                  return (
                    <Grid size={{ xs: 12, sm: 6, md: 4, lg: 3 }} key={item.uid}>
                      <Box
                        sx={{
                          width: '100%',
                          minWidth: 0,
                          boxSizing: 'border-box',
                          // FORK: ProfileBox selected cards shift 3px left; keep them inside grid columns.
                          pl: '3px',
                        }}
                      >
                        <SortableProfileItem
                          id={item.uid!}
                          selected={true}
                          activating={
                            activatings.includes(item.uid!) ||
                            visibleSwitchingProfile === item.uid
                          }
                          itemData={item}
                          timerUpdateRevision={
                            timerUpdateRevisions.get(item.uid!) ?? 0
                          }
                          completedUpdateRevision={
                            completedUpdateRevisions.get(item.uid!) ?? 0
                          }
                          mutateProfiles={mutateProfiles}
                          onSelect={() => onToggleProfile(item.uid!)}
                          onEdit={() => viewerRef.current?.edit(item)}
                          onSave={async (prev, curr) => {
                            if (
                              prev !== curr &&
                              profiles.current === item.uid
                            ) {
                              await onEnhance(false)
                            }
                          }}
                          onDelete={() => onDelete(item.uid!)}
                          batchMode={batchMode}
                          isSelected={batchSelected.has(item.uid!)}
                          onSelectionChange={() =>
                            toggleProfileSelection(item.uid!)
                          }
                        />
                      </Box>
                    </Grid>
                  )
                })}
              </SortableContext>
            </Grid>
          </Box>
          {inactiveProfiles.length > 0 && (
            <>
              <Divider
                variant="middle"
                flexItem
                sx={{
                  width: `calc(100% - 32px)`,
                  borderColor: dividercolor,
                  mb: 1.5,
                }}
              />
              <Box sx={{ mb: 1.5 }}>
                <Grid container spacing={{ xs: 1, lg: 1 }}>
                  {inactiveProfiles.map((item) => (
                    <Grid size={{ xs: 12, sm: 6, md: 4, lg: 3 }} key={item.uid}>
                      <SortableProfileItem
                        id={item.uid!}
                        selected={false}
                        activating={activatings.includes(item.uid!)}
                        itemData={item}
                        timerUpdateRevision={
                          timerUpdateRevisions.get(item.uid!) ?? 0
                        }
                        completedUpdateRevision={
                          completedUpdateRevisions.get(item.uid!) ?? 0
                        }
                        mutateProfiles={mutateProfiles}
                        onSelect={() => onToggleProfile(item.uid!)}
                        onEdit={() => viewerRef.current?.edit(item)}
                        onSave={async (prev, curr) => {
                          if (prev !== curr && profiles.current === item.uid) {
                            await onEnhance(false)
                          }
                        }}
                        onDelete={() => onDelete(item.uid!)}
                        batchMode={batchMode}
                        isSelected={batchSelected.has(item.uid!)}
                        onSelectionChange={() =>
                          toggleProfileSelection(item.uid!)
                        }
                      />
                    </Grid>
                  ))}
                </Grid>
              </Box>
            </>
          )}
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
        <DragOverlay />
      </DndContext>

      <ProfileViewer
        ref={viewerRef}
        onChange={async (isActivating) => {
          mutateProfiles()
          // 只有更改当前激活的配置时才触发全局重新加载
          if (isActivating) {
            await onEnhance(false)
          }
        }}
      />
      <ConfigViewer ref={configRef} />
      {/* FORK: Conflict viewer dialog for multi-profile merge */}
      <ConflictViewer
        open={conflictViewerOpen}
        conflicts={conflicts}
        onClose={() => setConflictViewerOpen(false)}
      />
    </BasePage>
  )
}

export default ProfilePage
