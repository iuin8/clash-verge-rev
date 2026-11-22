import { AddRounded, RefreshRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  IconButton,
  Paper,
  Skeleton,
  Stack,
  Tooltip,
  Typography,
} from '@mui/material'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate } from 'react-router'

import { BaseEmpty, BasePage } from '@/components/base'
import { OpenP2PAccountCard } from '@/components/openp2p/account-card'
import { OpenP2PArtifactLists } from '@/components/openp2p/artifact-lists'
import { OpenP2PImportDialog } from '@/components/openp2p/import-dialog'
import { useProfiles } from '@/hooks/use-profiles'
import {
  getOpenP2pState,
  previewOpenP2pConfig,
  readProfileFile,
  syncOpenP2pArtifacts,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { errorDetail, showNotice } from '@/services/notice-service'
import { revalidateQuery, useQuery } from '@/services/query-client'

const TRANSITION_PHASES = new Set([
  'starting',
  'loginPending',
  'degraded',
  'backoff',
  'stopping',
])

const OpenP2PPage = () => {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { profiles, current } = useProfiles()
  const [importOpen, setImportOpen] = useState(false)
  const [syncing, setSyncing] = useState(false)
  const syncingRef = useRef(false)
  const [manualRevision, setManualRevision] = useState(0)

  const stateQuery = useQuery({
    queryKey: ['getOpenP2pState'],
    queryFn: getOpenP2pState,
    staleTime: 1000,
    retry: 2,
    refetchInterval: (data) =>
      data?.accounts.some((account) => TRANSITION_PHASES.has(account.phase))
        ? 3000
        : 30000,
    refetchOnWindowFocus: true,
  })

  const view = stateQuery.data

  const liveProfileUid = profiles?.current ?? null
  const mergeUid = current?.option?.merge ?? null

  const sourceQuery = useQuery({
    queryKey: [
      'openp2p-current-source',
      liveProfileUid,
      mergeUid,
      view?.configRevision ?? 0,
      manualRevision,
    ],
    enabled: Boolean(mergeUid),
    queryFn: async () => {
      if (!mergeUid) return null
      const raw = await readProfileFile(mergeUid)
      if (!raw.trim() || !raw.includes('x-openp2p')) return null
      return previewOpenP2pConfig(raw)
    },
    refetchInterval: false,
    retry: 1,
    staleTime: 5000,
  })

  const artifacts = view?.artifacts
  const generated = sourceQuery.data ?? null
  const proxies = generated?.proxies ?? artifacts?.proxies ?? []
  const groups = generated?.groups ?? artifacts?.groups ?? []
  const rules = generated?.rules ?? []
  const dnsPolicy = generated?.dnsPolicy ?? []
  const hasArtifacts =
    proxies.length + groups.length + rules.length + dnsPolicy.length > 0
  const appliedUid = artifacts?.profileUid ?? null
  const profileMismatch =
    Boolean(liveProfileUid) &&
    liveProfileUid !== appliedUid &&
    ((view?.accounts.length ?? 0) > 0 || hasArtifacts)
  const showArtifacts =
    Boolean(view) &&
    ((view?.accounts.length ?? 0) > 0 ||
      hasArtifacts ||
      Boolean(artifacts?.error) ||
      profileMismatch)

  const refresh = async () => {
    setManualRevision((previous) => previous + 1)
    await Promise.all([stateQuery.refetch(), revalidateQuery(['getProfiles'])])
  }

  const applyCurrentArtifacts = async () => {
    if (syncingRef.current) return
    syncingRef.current = true
    setSyncing(true)
    try {
      const result = await mutate(syncOpenP2pArtifacts, {
        id: 'openp2p:sync-artifacts',
        revalidate: [['getOpenP2pState']],
        errorNotice: false,
      })
      if (!result.ok) {
        showNotice.error(t('shared.feedback.errors.operationFailed'))
        return
      }
      setManualRevision((previous) => previous + 1)
      showNotice.success(t('openp2p.artifacts.applySuccess'))
    } catch (cause) {
      showNotice.error(cause)
    } finally {
      syncingRef.current = false
      setSyncing(false)
    }
  }

  return (
    <BasePage
      title={t('openp2p.page.title')}
      contentStyle={{ padding: 2 }}
      header={
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
          <Button
            size="small"
            variant="contained"
            startIcon={<AddRounded />}
            onClick={() => setImportOpen(true)}
          >
            {t('openp2p.page.actions.import')}
          </Button>
          <Tooltip title={t('openp2p.page.actions.refresh')}>
            <span>
              <IconButton
                size="small"
                color="inherit"
                disabled={stateQuery.isFetching}
                onClick={() => void refresh()}
                aria-label={t('openp2p.page.actions.refresh')}
              >
                {stateQuery.isFetching ? (
                  <CircularProgress size={18} color="inherit" />
                ) : (
                  <RefreshRounded />
                )}
              </IconButton>
            </span>
          </Tooltip>
        </Stack>
      }
    >
      <Stack spacing={1.5}>
        {stateQuery.isPending && !view && (
          <>
            <Skeleton variant="rounded" height={180} />
            <Skeleton variant="rounded" height={180} />
          </>
        )}

        {stateQuery.error && !view && (
          <Alert
            severity="error"
            action={
              <Button
                size="small"
                color="inherit"
                onClick={() => void refresh()}
              >
                {t('shared.actions.retry')}
              </Button>
            }
          >
            <Typography variant="subtitle2">
              {t('openp2p.page.error.title')}
            </Typography>
            <Typography variant="body2">
              {errorDetail(stateQuery.error) ||
                t('shared.feedback.errors.unexpected')}
            </Typography>
          </Alert>
        )}

        {stateQuery.error && view && (
          <Alert severity="warning">
            {errorDetail(stateQuery.error) ||
              t('shared.feedback.errors.unexpected')}
          </Alert>
        )}

        {view?.configError && (
          <Alert severity="error">
            <Typography variant="subtitle2">
              {t('openp2p.page.configError')}
            </Typography>
            <Typography
              variant="body2"
              sx={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}
            >
              {view.configError}
            </Typography>
          </Alert>
        )}

        {view && view.accounts.length === 0 && (
          <Box sx={{ display: 'flex', minHeight: 360 }}>
            <BaseEmpty
              text={t('openp2p.page.empty.title')}
              extra={
                <Stack
                  spacing={1}
                  sx={{ alignItems: 'center', maxWidth: 460, mt: 1 }}
                >
                  <Typography
                    variant="body2"
                    color="text.secondary"
                    align="center"
                  >
                    {t('openp2p.page.empty.description')}
                  </Typography>
                  <Button
                    variant="contained"
                    startIcon={<AddRounded />}
                    onClick={() => setImportOpen(true)}
                  >
                    {t('openp2p.page.actions.import')}
                  </Button>
                </Stack>
              }
            />
          </Box>
        )}

        {view && view.accounts.length > 0 && (
          <Stack spacing={1.5}>
            {view.accounts.map((account) => (
              <OpenP2PAccountCard key={account.id} account={account} />
            ))}
          </Stack>
        )}

        {showArtifacts && view && (
          <Paper variant="outlined" sx={{ p: 2, borderRadius: 2 }}>
            <Stack spacing={1.5}>
              <Stack
                direction="row"
                spacing={1}
                sx={{
                  alignItems: 'flex-start',
                  justifyContent: 'space-between',
                  flexWrap: 'wrap',
                }}
              >
                <Box sx={{ minWidth: 0 }}>
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                  >
                    <Typography variant="h6" sx={{ fontWeight: 600 }}>
                      {t('openp2p.artifacts.title')}
                    </Typography>
                    <Chip
                      size="small"
                      variant="outlined"
                      color={artifacts?.enhanced ? 'success' : 'default'}
                      label={
                        artifacts?.enhanced
                          ? t('openp2p.artifacts.applied')
                          : t('openp2p.artifacts.notApplied')
                      }
                    />
                    {sourceQuery.isFetching && <CircularProgress size={14} />}
                  </Stack>
                  <Typography variant="caption" color="text.secondary">
                    {t('openp2p.artifacts.description')}
                  </Typography>
                </Box>
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{ alignItems: 'center' }}
                >
                  <Button
                    size="small"
                    variant="outlined"
                    onClick={() => navigate('/profile')}
                  >
                    {t('openp2p.artifacts.openProfiles')}
                  </Button>
                  <Button
                    size="small"
                    variant="contained"
                    loading={syncing}
                    disabled={!liveProfileUid || syncing}
                    onClick={() => void applyCurrentArtifacts()}
                  >
                    {t('openp2p.artifacts.applyCurrent')}
                  </Button>
                </Stack>
              </Stack>

              {!liveProfileUid && (
                <Typography variant="body2" color="text.secondary">
                  {t('openp2p.artifacts.noProfile')}
                </Typography>
              )}

              {profileMismatch && (
                <Alert severity="warning">
                  <Typography variant="subtitle2">
                    {t('openp2p.artifacts.mismatchTitle')}
                  </Typography>
                  <Typography variant="body2">
                    {t('openp2p.artifacts.mismatchDescription')}
                  </Typography>
                </Alert>
              )}

              {artifacts?.error && (
                <Alert severity="error">
                  <Typography variant="subtitle2">
                    {t('openp2p.artifacts.error')}
                  </Typography>
                  <Typography variant="body2">{artifacts.error}</Typography>
                </Alert>
              )}

              {sourceQuery.error && (
                <Alert severity="warning">
                  <Typography variant="subtitle2">
                    {t('openp2p.artifacts.sourceError')}
                  </Typography>
                  <Typography variant="body2">
                    {errorDetail(sourceQuery.error)}
                  </Typography>
                </Alert>
              )}

              {hasArtifacts ? (
                <OpenP2PArtifactLists
                  proxies={proxies}
                  groups={groups}
                  rules={rules}
                  dnsPolicy={dnsPolicy}
                />
              ) : (
                <Typography variant="body2" color="text.disabled">
                  {t('openp2p.artifacts.empty')}
                </Typography>
              )}
            </Stack>
          </Paper>
        )}
      </Stack>

      {importOpen && (
        <OpenP2PImportDialog onClose={() => setImportOpen(false)} />
      )}
    </BasePage>
  )
}

export default OpenP2PPage
