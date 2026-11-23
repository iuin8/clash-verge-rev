import { AddRounded, RefreshRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  IconButton,
  Skeleton,
  Stack,
  Tooltip,
  Typography,
} from '@mui/material'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseEmpty, BasePage } from '@/components/base'
import { OpenP2PAccountCard } from '@/components/openp2p/account-card'
import { translateOpenP2pErrorText } from '@/components/openp2p/errors'
import { OpenP2PImportDialog } from '@/components/openp2p/import-dialog'
import { OpenP2PSourceCard } from '@/components/openp2p/source-card'
import type { OpenP2pState } from '@/services/cmds'
import { getOpenP2pState } from '@/services/cmds'
import { errorDetail } from '@/services/notice-service'
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
  const [importOpen, setImportOpen] = useState(false)

  const stateQuery = useQuery<OpenP2pState>({
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

  const refresh = async () => {
    await Promise.all([stateQuery.refetch(), revalidateQuery(['getProfiles'])])
  }

  const handleImported = () => {
    setImportOpen(false)
    void refresh()
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
            <Skeleton variant="rounded" height={150} />
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
              {translateOpenP2pErrorText(view.configError, t)}
            </Typography>
          </Alert>
        )}

        {view && (
          <OpenP2PSourceCard
            source={view.source}
            accountCount={view.accounts.length}
          />
        )}

        {view && view.accounts.length === 0 && (
          <Box sx={{ display: 'flex', minHeight: 340 }}>
            <BaseEmpty
              text={t('openp2p.page.empty.title')}
              extra={
                <Stack
                  spacing={1}
                  sx={{ alignItems: 'center', maxWidth: 520, mt: 1 }}
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
              <OpenP2PAccountCard
                key={account.id}
                account={account}
                source={view.source}
              />
            ))}
          </Stack>
        )}
      </Stack>

      {importOpen && (
        <OpenP2PImportDialog
          onClose={() => setImportOpen(false)}
          onApplied={handleImported}
        />
      )}
    </BasePage>
  )
}

export default OpenP2PPage
