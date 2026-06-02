import {
  closestCenter,
  DndContext,
  type DragEndEvent,
  type DragStartEvent,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import {
  arrayMove,
  rectSortingStrategy,
  SortableContext,
  sortableKeyboardCoordinates,
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
} from '@mui/icons-material'
import { Box, Button, Divider, Grid, IconButton, Stack } from '@mui/material'
import { useQuery } from '@tanstack/react-query'
import { listen, TauriEvent } from '@tauri-apps/api/event'
import { readText } from '@tauri-apps/plugin-clipboard-manager'
import { readTextFile } from '@tauri-apps/plugin-fs'
import { useLockFn } from 'ahooks'
import { throttle } from 'lodash-es'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from 'react'
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
import { useProfiles } from '@/hooks/use-profiles'
import {
  clearMergedProfiles,
  createProfile,
  deleteProfile,
  enhanceProfiles,
  getProfiles,
  //restartCore,
  getMergeConflicts,
  getRuntimeLogs,
  importProfile,
  reorderProfile,
  setMergedProfiles,
  updateProfile,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import { queryClient } from '@/services/query-client'
import { useSetLoadingCache, useThemeMode } from '@/services/states'
import { debugLog } from '@/utils/debug'

// 记录profile切换状态
const debugProfileSwitch = (action: string, profile: string, extra?: any) => {
  const timestamp = new Date().toISOString().substring(11, 23)
  debugLog(`[Profile-Debug][${timestamp}] ${action}: ${profile}`, extra || '')
}

// 检查请求是否已过期
const isRequestOutdated = (
  currentSequence: number,
  requestSequenceRef: RefObject<number>,
  profile: string,
) => {
  if (currentSequence !== requestSequenceRef.current) {
    debugProfileSwitch(
      'REQUEST_OUTDATED',
      profile,
      `当前序列号: ${currentSequence}, 最新序列号: ${requestSequenceRef.current}`,
    )
    return true
  }
  return false
}

// 检查是否被中断
const isOperationAborted = (
  abortController: AbortController,
  profile: string,
) => {
  if (abortController.signal.aborted) {
    debugProfileSwitch('OPERATION_ABORTED', profile)
    return true
  }
  return false
}

const ProfilePage = () => {
  const { t } = useTranslation()
  const location = useLocation()
  const { addListener } = useListen()
  const [url, setUrl] = useState('')
  const [disabled, setDisabled] = useState(false)
  const [activatings, setActivatings] = useState<string[]>([])
  const [loading, setLoading] = useState(false)

  const [draggingId, setDraggingId] = useState<string | null>(null)
  // FORK: Local drag order — decoupled from SWR to prevent revalidation snap-back
  const [localActiveOrder, setLocalActiveOrder] = useState<string[]>([])

  // FORK: Multi-profile merge state — selectedProfiles always mirrors active state
  const [selectedProfiles, setSelectedProfiles] = useState<Set<string>>(
    () => new Set(),
  )
  // 上游 v2.5.1 batch-select：批量删除模式的勾选集（独立于多激活 selectedProfiles）
  const [batchMode, setBatchMode] = useState(false)
  const [batchSelected, setBatchSelected] = useState<Set<string>>(
    () => new Set(),
  )
  const [conflictViewerOpen, setConflictViewerOpen] = useState(false)
  const [conflicts, setConflicts] = useState<ConflictEntry[]>([])

  // 防止重复切换
  const switchingProfileRef = useRef<string | null>(null)

  // 支持中断当前切换操作
  const abortControllerRef = useRef<AbortController | null>(null)

  // 只处理最新的切换请求
  const requestSequenceRef = useRef<number>(0)

  // 待处理请求跟踪，取消排队的请求
  const pendingRequestRef = useRef<Promise<any> | null>(null)

  // 防止 toggle 进行中 hydration effect 回写旧状态
  const isTogglingRef = useRef(false)

  // 处理profile切换中断
  const handleProfileInterrupt = useCallback(
    (previousSwitching: string, newProfile: string) => {
      debugProfileSwitch(
        'INTERRUPT_PREVIOUS',
        previousSwitching,
        `被 ${newProfile} 中断`,
      )

      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
        debugProfileSwitch('ABORT_CONTROLLER_TRIGGERED', previousSwitching)
      }

      if (pendingRequestRef.current) {
        debugProfileSwitch('CANCEL_PENDING_REQUEST', previousSwitching)
      }

      setActivatings((prev) => prev.filter((id) => id !== previousSwitching))
      showNotice.info(
        'profiles.page.feedback.notifications.switchInterrupted',
        `${previousSwitching} → ${newProfile}`,
        3000,
      )
    },
    [],
  )

  // 清理切换状态
  const cleanupSwitchState = useCallback(
    (profile: string, sequence: number) => {
      setActivatings((prev) => prev.filter((id) => id !== profile))
      switchingProfileRef.current = null
      abortControllerRef.current = null
      pendingRequestRef.current = null
      debugProfileSwitch('SWITCH_END', profile, `序列号: ${sequence}`)
    },
    [],
  )
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
    activateSelected,
    patchProfiles,
    mutateProfiles,
    error,
    isStale,
  } = useProfiles()

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
  }, [addListener, mutateProfiles, t])

  // 添加紧急恢复功能
  const onEmergencyRefresh = useLockFn(async () => {
    debugLog('[紧急刷新] 开始强制刷新所有数据')

    try {
      // 只失效 profiles 相关 query，不影响 WS 订阅、IP 缓存等其他 query
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['getProfiles'] }),
        queryClient.invalidateQueries({ queryKey: ['getRuntimeLogs'] }),
      ])

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

  const { data: chainLogs = {}, refetch: mutateLogs } = useQuery({
    queryKey: ['getRuntimeLogs'],
    queryFn: getRuntimeLogs,
  })

  const viewerRef = useRef<ProfileViewerRef>(null)
  const configRef = useRef<DialogRef>(null)

  // distinguish type
  const profileItems = useMemo(() => {
    const items = profiles.items || []

    const type1 = ['local', 'remote']

    return items.filter((i) => i && type1.includes(i.type!))
  }, [profiles])

  const activeProfiles = useMemo(() => {
    // During drag: use localActiveOrder for snap-back-free reordering
    // Otherwise: derive from selectedProfiles, preserving any prior local order
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

  const primaryUid =
    selectedProfiles.size >= 2 ? (activeProfiles[0]?.uid ?? null) : null

  const currentActivatings = profiles.current ? [profiles.current] : []

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
      await queryClient.fetchQuery({
        queryKey: ['getProfiles'],
        queryFn: getProfiles,
      })
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
    // Seed localActiveOrder on first drag so active zone never collapses to empty
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

    // Only active-zone reordering — inactive cards are not draggable.
    // localActiveOrder 在 onDragStart 才被种子，但 React state 异步生效，
    // 此处可能仍为空数组；fallback 到 activeProfiles 实时计算的顺序。
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
      // 后端 reorder 同步重排 merged 数组,但 SWR 缓存的 profiles.merged
      // 还是旧顺序。切 tab 后 hydration useEffect 会用 SWR 缓存重置
      // selectedProfiles → 顺序回退。这里显式 mutate 让缓存同步到磁盘真值。
      await mutateProfiles()
    } catch {
      setLocalActiveOrder(oldOrder)
    }
  }

  const executeBackgroundTasks = useCallback(
    async (
      profile: string,
      sequence: number,
      abortController: AbortController,
    ) => {
      try {
        if (
          sequence === requestSequenceRef.current &&
          switchingProfileRef.current === profile &&
          !abortController.signal.aborted
        ) {
          await activateSelected(profiles)
          debugLog(`[Profile] 后台处理完成，序列号: ${sequence}`)
        } else {
          debugProfileSwitch(
            'BACKGROUND_TASK_SKIPPED',
            profile,
            `序列号过期或被中断: ${sequence} vs ${requestSequenceRef.current}`,
          )
        }
      } catch (err: any) {
        console.warn('Failed to activate selected proxies:', err)
      }
    },
    [activateSelected, profiles],
  )

  const activateProfile = useCallback(
    async (profile: string, notifySuccess: boolean) => {
      if (profiles.current === profile && !notifySuccess) {
        debugLog(`[Profile] 目标profile ${profile} 已经是当前配置，跳过切换`)
        return
      }

      const currentSequence = ++requestSequenceRef.current
      debugProfileSwitch('NEW_REQUEST', profile, `序列号: ${currentSequence}`)

      // 处理中断逻辑
      const previousSwitching = switchingProfileRef.current
      if (previousSwitching && previousSwitching !== profile) {
        handleProfileInterrupt(previousSwitching, profile)
      }

      // 防止重复切换同一个profile
      if (switchingProfileRef.current === profile) {
        debugProfileSwitch('DUPLICATE_SWITCH_BLOCKED', profile)
        return
      }

      // 初始化切换状态
      switchingProfileRef.current = profile
      debugProfileSwitch('SWITCH_START', profile, `序列号: ${currentSequence}`)

      const currentAbortController = new AbortController()
      abortControllerRef.current = currentAbortController

      setActivatings((prev) => {
        if (prev.includes(profile)) return prev
        return [...prev, profile]
      })

      try {
        debugLog(`[Profile] 开始切换到: ${profile}，序列号: ${currentSequence}`)

        // 检查请求有效性
        if (
          isRequestOutdated(currentSequence, requestSequenceRef, profile) ||
          isOperationAborted(currentAbortController, profile)
        ) {
          return
        }

        // 执行切换请求
        const requestPromise = patchProfiles(
          { current: profile },
          currentAbortController.signal,
          {
            deferRefreshOnSuccess: true,
          },
        )
        pendingRequestRef.current = requestPromise

        const success = await requestPromise

        if (pendingRequestRef.current === requestPromise) {
          pendingRequestRef.current = null
        }

        // 再次检查有效性
        if (
          isRequestOutdated(currentSequence, requestSequenceRef, profile) ||
          isOperationAborted(currentAbortController, profile)
        ) {
          return
        }

        // 后端返回 success=false 表示验证/更新失败，已 discard draft 并 restore previous。
        // 此时不能 closeAllConnections / 触发后台代理切换，否则会无故掐断用户连接并执行无效刷新。
        if (!success) {
          await mutateProfiles()
          if (notifySuccess) {
            showNotice.error(
              'profiles.page.feedback.notifications.profileSwitchFailed',
              4000,
            )
          }
          debugLog(
            `[Profile] 切换到 ${profile} 失败 (success=false)，跳过连接关闭与后台任务`,
          )
          return
        }

        // 完成切换
        await mutateLogs()
        closeAllConnections()

        if (notifySuccess) {
          showNotice.success(
            'profiles.page.feedback.notifications.profileSwitched',
            1000,
          )
        }

        debugLog(
          `[Profile] 切换到 ${profile} 完成，序列号: ${currentSequence}，开始后台处理`,
        )

        // 延迟执行后台任务
        setTimeout(
          () =>
            executeBackgroundTasks(
              profile,
              currentSequence,
              currentAbortController,
            ),
          50,
        )
      } catch (err: any) {
        if (pendingRequestRef.current) {
          pendingRequestRef.current = null
        }

        // 检查是否因为中断或过期而出错
        if (
          isOperationAborted(currentAbortController, profile) ||
          isRequestOutdated(currentSequence, requestSequenceRef, profile)
        ) {
          return
        }

        console.error(`[Profile] 切换失败:`, err)
        showNotice.error(err, 4000)
      } finally {
        // 只有当前profile仍然是正在切换的profile且序列号匹配时才清理状态
        if (
          switchingProfileRef.current === profile &&
          currentSequence === requestSequenceRef.current
        ) {
          cleanupSwitchState(profile, currentSequence)
        } else {
          debugProfileSwitch(
            'CLEANUP_SKIPPED',
            profile,
            `序列号不匹配或已被接管: ${currentSequence} vs ${requestSequenceRef.current}`,
          )
        }
      }
    },
    [
      profiles,
      patchProfiles,
      mutateLogs,
      mutateProfiles,
      executeBackgroundTasks,
      handleProfileInterrupt,
      cleanupSwitchState,
    ],
  )

  useEffect(() => {
    ;(async () => {
      if (current) {
        mutateProfiles()
        await activateProfile(current, false)
      }
    })()
  }, [current, activateProfile, mutateProfiles])

  const onEnhance = useLockFn(async (notifySuccess: boolean) => {
    if (switchingProfileRef.current) {
      debugLog(
        `[Profile] 有profile正在切换中(${switchingProfileRef.current})，跳过enhance操作`,
      )
      return
    }

    const currentProfiles = currentActivatings
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
      // 保留正在切换的profile，清除其他状态
      setActivatings((prev) =>
        prev.filter((id) => id === switchingProfileRef.current),
      )
    }
  })

  const onDelete = useLockFn(async (uid: string) => {
    const current = profiles.current === uid
    try {
      setActivatings([...(current ? currentActivatings : []), uid])
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
  const setLoadingCache = useSetLoadingCache()
  const onUpdateAll = useLockFn(async () => {
    const throttleMutate = throttle(mutateProfiles, 2000, {
      trailing: true,
    })
    const updateOne = async (uid: string) => {
      try {
        await updateProfile(uid)
        throttleMutate()
      } catch (err: any) {
        console.error(`更新订阅 ${uid} 失败:`, err)
      } finally {
        setLoadingCache((cache) => ({ ...cache, [uid]: false }))
      }
    }

    return new Promise((resolve) => {
      setLoadingCache((cache) => {
        // 获取没有正在更新的订阅
        const items = profileItems.filter(
          (e) => e.type === 'remote' && !cache[e.uid],
        )
        const change = Object.fromEntries(items.map((e) => [e.uid, true]))

        Promise.allSettled(items.map((e) => updateOne(e.uid))).then(resolve)
        return { ...cache, ...change }
      })
    })
  })

  const onCopyLink = async () => {
    const text = await readText()
    if (text) setUrl(text)
  }

  const onToggleProfile = useLockFn(async (uid: string) => {
    const newSet = new Set(selectedProfiles)
    if (newSet.has(uid)) {
      if (newSet.size <= 1) return // must keep at least 1 active
      newSet.delete(uid)
    } else {
      newSet.add(uid)
    }
    const prevSet = selectedProfiles
    isTogglingRef.current = true
    setSelectedProfiles(newSet)
    try {
      if (newSet.size === 1) {
        const singleUid = [...newSet][0]
        // patchProfiles 在后端验证/更新失败时返回 false 而非抛异常 (use-profiles.ts)。
        // 必须显式回滚乐观更新，否则 UI 看着像启用了，切 tab 回来 hydration
        // useEffect 会把 selectedProfiles 重置为后端真值，造成"开关状态突变"。
        const success = await patchProfiles({ current: singleUid })
        if (!success) {
          setSelectedProfiles(prevSet)
          showNotice.error(
            'profiles.page.feedback.notifications.profileSwitchFailed',
            4000,
          )
          return
        }
        await clearMergedProfiles()
      } else {
        await setMergedProfiles([...newSet])
        const c = await getMergeConflicts()
        setConflicts(c)
      }
    } catch (err: any) {
      setSelectedProfiles(prevSet)
      showNotice.error(err)
    } finally {
      isTogglingRef.current = false
    }
  })

  // Hydrate selectedProfiles from persisted backend state
  const mergedUidsKey = useMemo(
    () => (profiles?.merged ?? []).join(','),
    [profiles?.merged],
  )
  const currentUid = profiles?.current ?? ''
  useEffect(() => {
    if (isTogglingRef.current) return
    const uids = mergedUidsKey ? mergedUidsKey.split(',') : []
    if (uids.length >= 2) {
      void Promise.resolve().then(() => {
        setSelectedProfiles(new Set(uids))
      })
      getMergeConflicts()
        .then(setConflicts)
        .catch((e) => console.error('[merge] failed to load conflicts', e))
    } else if (currentUid) {
      void Promise.resolve().then(() => {
        setSelectedProfiles(new Set([currentUid]))
      })
    }
  }, [mergedUidsKey, currentUid])

  const mode = useThemeMode()
  const isLight = mode === 'light'
  const dividercolor = isLight
    ? 'rgba(0, 0, 0, 0.06)'
    : 'rgba(255, 255, 255, 0.06)'

  // 监听后端配置变更
  useEffect(() => {
    let unlistenPromise: Promise<() => void> | undefined
    let lastProfileId: string | null = null
    let lastUpdateTime = 0
    const debounceDelay = 200

    let refreshTimer: number | null = null

    const setupListener = async () => {
      unlistenPromise = listen<string>('profile-changed', (event) => {
        const newProfileId = event.payload
        const now = Date.now()

        debugLog(`[Profile] 收到配置变更事件: ${newProfileId}`)

        if (
          lastProfileId === newProfileId &&
          now - lastUpdateTime < debounceDelay
        ) {
          debugLog(`[Profile] 重复事件被防抖，跳过`)
          return
        }

        lastProfileId = newProfileId
        lastUpdateTime = now

        debugLog(`[Profile] 执行配置数据刷新`)

        if (refreshTimer !== null) {
          window.clearTimeout(refreshTimer)
        }

        // 使用异步调度避免阻塞事件处理
        refreshTimer = window.setTimeout(() => {
          mutateProfiles().catch((error) => {
            console.error('[Profile] 配置数据刷新失败:', error)
          })
          refreshTimer = null
        }, 0)
      })
    }

    setupListener()

    return () => {
      if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer)
      }
      unlistenPromise?.then((unlisten) => unlisten()).catch(console.error)
    }
  }, [mutateProfiles])

  // 组件卸载时清理中断控制器
  useEffect(() => {
    return () => {
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
        debugProfileSwitch('COMPONENT_UNMOUNT_CLEANUP', 'all')
      }
    }
  }, [])

  // ── 上游 v2.5.1 batch-select：批量删除（与 fork 多激活合并并存的可切换模式）──
  const toggleBatchMode = () => {
    setBatchMode((prev) => !prev)
    // 进入批量模式时清空之前的勾选
    if (!batchMode) {
      setBatchSelected(new Set())
    }
  }
  const toggleBatchSelection = (uid: string) => {
    setBatchSelected((prev) => {
      const next = new Set(prev)
      if (next.has(uid)) {
        next.delete(uid)
      } else {
        next.add(uid)
      }
      return next
    })
  }
  const selectAllProfiles = () => {
    setBatchSelected(new Set(profileItems.map((item) => item.uid!)))
  }
  const clearAllSelections = () => {
    setBatchSelected(new Set())
  }
  const isAllSelected = () =>
    profileItems.length > 0 && profileItems.length === batchSelected.size
  const getSelectionState = (): 'none' | 'partial' | 'all' => {
    if (batchSelected.size === 0) return 'none'
    if (batchSelected.size === profileItems.length) return 'all'
    return 'partial'
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
      // 若删除的包含当前激活 profile，重跑 enhance pipeline
      if (currentActivating.length > 0) {
        await onEnhance(false)
      }
      setBatchSelected(new Set())
      setBatchMode(false)
      showNotice.success('profiles.page.feedback.notifications.batchDeleted')
    } catch (err) {
      showNotice.error(err)
    } finally {
      setActivatings([])
    }
  })

  return (
    <BasePage
      full
      title={t('profiles.page.title')}
      contentStyle={{ height: '100%' }}
      header={
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 1 }}>
          {!batchMode ? (
            <>
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
            <Grid container spacing={1}>
              <SortableContext
                items={activeProfiles.map((p) => p.uid!)}
                strategy={rectSortingStrategy}
              >
                {activeProfiles.map((item) => {
                  const isPrimary = primaryUid === item.uid
                  return (
                    <Grid size={{ xs: 12, sm: 6, md: 4, lg: 3 }} key={item.uid}>
                      <ProfileItem
                        id={item.uid}
                        selected={true}
                        draggable={true}
                        activating={activatings.includes(item.uid)}
                        itemData={item}
                        mutateProfiles={mutateProfiles}
                        onEdit={() => viewerRef.current?.edit(item)}
                        onSave={async (prev, curr) => {
                          if (prev !== curr && profiles.current === item.uid) {
                            await onEnhance(false)
                          }
                        }}
                        onDelete={() => onDelete(item.uid)}
                        isPrimary={isPrimary}
                        conflictCount={isPrimary ? conflicts.length : 0}
                        onShowConflicts={() => setConflictViewerOpen(true)}
                        onToggle={() => onToggleProfile(item.uid!)}
                        batchMode={batchMode}
                        isSelected={batchSelected.has(item.uid!)}
                        onSelectionChange={() =>
                          toggleBatchSelection(item.uid!)
                        }
                      />
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
                <Grid container spacing={1}>
                  {inactiveProfiles.map((item) => (
                    <Grid size={{ xs: 12, sm: 6, md: 4, lg: 3 }} key={item.uid}>
                      <ProfileItem
                        id={item.uid}
                        selected={false}
                        draggable={false}
                        activating={activatings.includes(item.uid)}
                        itemData={item}
                        mutateProfiles={mutateProfiles}
                        onEdit={() => viewerRef.current?.edit(item)}
                        onSave={async (prev, curr) => {
                          if (prev !== curr && profiles.current === item.uid) {
                            await onEnhance(false)
                          }
                        }}
                        onDelete={() => onDelete(item.uid)}
                        isPrimary={false}
                        conflictCount={0}
                        onShowConflicts={() => setConflictViewerOpen(true)}
                        onToggle={() => onToggleProfile(item.uid!)}
                        batchMode={batchMode}
                        isSelected={batchSelected.has(item.uid!)}
                        onSelectionChange={() =>
                          toggleBatchSelection(item.uid!)
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
          />
          <Box sx={{ mt: 1.5, mb: '10px' }}>
            <Grid container spacing={1}>
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
