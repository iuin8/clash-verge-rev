import {
  DeleteOutlineRounded,
  FolderOpenOutlined,
  KeyOutlined,
  PlayArrowRounded,
  RefreshRounded,
  StopRounded,
  VpnKeyOutlined,
} from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog } from '@/components/base'
import type { OpenP2PAccount_Serialize as OpenP2PAccount } from '@/services/bindings'
import {
  deleteOpenP2pAccount,
  openOpenP2pLogDir,
  restartOpenP2pAccount,
  startOpenP2pAccount,
  stopOpenP2pAccount,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { revalidateQuery } from '@/services/query-client'
import type { TranslationKey } from '@/types/generated/i18n-keys'

import { OpenP2PKeyDialog } from './key-dialog'
import {
  OpenP2PKeyBadge,
  OpenP2PListenBadge,
  OpenP2PLoginBadge,
  OpenP2PPhaseBadge,
  OpenP2PTcpBadge,
  OpenP2PTokenBadge,
} from './status-badges'
import { OpenP2PTokenDialog } from './token-dialog'

const RUNNING_PHASES = new Set([
  'starting',
  'loginPending',
  'online',
  'degraded',
  'backoff',
  'stopping',
])

const WARNING_PHASES = new Set(['degraded', 'backoff', 'tokenMissing'])

const Detail = ({
  label,
  value,
}: {
  label: TranslationKey
  value: string | number
}) => {
  const { t } = useTranslation()
  return (
    <Box sx={{ minWidth: 0 }}>
      <Typography variant="caption" color="text.secondary">
        {t(label)}
      </Typography>
      <Typography
        variant="body2"
        sx={{
          fontFamily: 'monospace',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
        title={String(value)}
      >
        {value === '' ? '—' : value}
      </Typography>
    </Box>
  )
}

export const OpenP2PAccountCard = ({
  account,
}: {
  account: OpenP2PAccount
}) => {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const [tokenOpen, setTokenOpen] = useState(false)
  const [keyOpen, setKeyOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)

  const running = RUNNING_PHASES.has(account.phase)
  const hasKeyNode = account.keyRefs.length > 0

  const runAction = async (
    name: string,
    action: () => Promise<unknown>,
    revalidate = true,
  ) => {
    if (busyRef.current) return false
    busyRef.current = true
    setBusy(true)
    try {
      const result = await mutate(action, {
        id: `openp2p:${name}:${account.id}`,
        ...(revalidate ? { revalidate: [['getOpenP2pState']] } : {}),
      })
      return result.ok
    } catch {
      if (revalidate) {
        await revalidateQuery(['getOpenP2pState'])
      }
      return false
    } finally {
      busyRef.current = false
      setBusy(false)
    }
  }

  const handleDelete = async () => {
    const deleted = await runAction('delete', () =>
      deleteOpenP2pAccount(account.id),
    )
    if (deleted) setDeleteOpen(false)
  }

  const actionTitle = !account.enabled
    ? t('openp2p.account.disabled')
    : !account.tokenPresent
      ? t('openp2p.account.tokenMissing')
      : undefined

  return (
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
                {account.label}
              </Typography>
              <Chip
                size="small"
                variant="outlined"
                label={account.id}
                sx={{ fontFamily: 'monospace' }}
              />
              {!account.enabled && (
                <Chip
                  size="small"
                  label={t('openp2p.account.disabled')}
                  variant="outlined"
                />
              )}
            </Stack>
            <Typography variant="caption" color="text.secondary">
              {account.gateway.host
                ? `${account.gateway.host}:${account.gateway.port}`
                : '—'}{' '}
              · {account.node || '—'}
            </Typography>
          </Box>
          <Stack direction="row" spacing={0.5} sx={{ alignItems: 'center' }}>
            {account.pid !== undefined && account.pid !== null && (
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ fontFamily: 'monospace' }}
              >
                PID {account.pid}
              </Typography>
            )}
            <OpenP2PPhaseBadge phase={account.phase} />
          </Stack>
        </Stack>

        <Box
          sx={{
            display: 'grid',
            gridTemplateColumns: {
              xs: 'repeat(2, minmax(0, 1fr))',
              md: 'repeat(4, minmax(0, 1fr))',
            },
            gap: 1,
          }}
        >
          <Detail
            label="openp2p.account.gateway"
            value={
              account.gateway.host
                ? `${account.gateway.host}:${account.gateway.port}`
                : ''
            }
          />
          <Detail label="openp2p.account.node" value={account.node} />
          <Detail
            label="openp2p.account.publicPort"
            value={account.publicIpPort}
          />
          <Detail label="openp2p.account.interface" value={account.interface} />
        </Box>

        <Stack
          direction="row"
          spacing={0.5}
          sx={{ alignItems: 'center', flexWrap: 'wrap' }}
        >
          <OpenP2PLoginBadge
            state={account.checks.login.state}
            detail={account.checks.login.detail}
          />
          <OpenP2PTokenBadge present={account.tokenPresent} />
          <OpenP2PKeyBadge present={account.keyPresent} hasNode={hasKeyNode} />
          {account.restartCount > 0 && (
            <Chip
              size="small"
              variant="outlined"
              label={t('openp2p.account.restarts', {
                value: account.restartCount,
              })}
            />
          )}
        </Stack>

        {account.lastError && (
          <Alert
            severity={WARNING_PHASES.has(account.phase) ? 'warning' : 'error'}
          >
            <Typography
              variant="body2"
              sx={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}
            >
              {account.lastError}
            </Typography>
          </Alert>
        )}

        <Box sx={{ border: 1, borderColor: 'divider', borderRadius: 1 }}>
          <Box sx={{ px: 1.25, py: 0.75 }}>
            <Typography variant="subtitle2">
              {t('openp2p.account.checks.apps')}
            </Typography>
          </Box>
          {account.checks.apps.length === 0 ? (
            <Typography
              variant="body2"
              color="text.disabled"
              sx={{ px: 1.25, pb: 1 }}
            >
              {t('openp2p.account.checks.noApps')}
            </Typography>
          ) : (
            account.checks.apps.map((app) => (
              <Box
                key={`${app.name}:${app.srcPort}`}
                sx={{
                  px: 1.25,
                  py: 0.75,
                  borderTop: 1,
                  borderColor: 'divider',
                }}
              >
                <Stack
                  direction="row"
                  spacing={1}
                  sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                >
                  <Typography variant="body2" sx={{ fontWeight: 600 }}>
                    {app.name}
                  </Typography>
                  <Chip
                    size="small"
                    variant="outlined"
                    label={app.protocol.toUpperCase()}
                  />
                  <Typography variant="body2" sx={{ fontFamily: 'monospace' }}>
                    127.0.0.1:{app.srcPort}
                  </Typography>
                  <OpenP2PListenBadge listening={app.listen} />
                  {app.protocol.toLowerCase() === 'tcp' && (
                    <OpenP2PTcpBadge reachable={app.tcpReachable} />
                  )}
                </Stack>
                {app.banner && (
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    noWrap
                    title={app.banner}
                    sx={{ fontFamily: 'monospace', display: 'block' }}
                  >
                    {app.banner}
                  </Typography>
                )}
              </Box>
            ))
          )}
        </Box>

        <Stack
          direction="row"
          spacing={1}
          sx={{ alignItems: 'center', flexWrap: 'wrap' }}
        >
          <Button
            size="small"
            variant="contained"
            startIcon={<PlayArrowRounded />}
            disabled={
              busy || running || !account.enabled || !account.tokenPresent
            }
            title={actionTitle}
            onClick={() =>
              void runAction('start', () => startOpenP2pAccount(account.id))
            }
          >
            {t('openp2p.account.actions.start')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            startIcon={<StopRounded />}
            disabled={busy || !running}
            onClick={() =>
              void runAction('stop', () => stopOpenP2pAccount(account.id))
            }
          >
            {t('openp2p.account.actions.stop')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            startIcon={<RefreshRounded />}
            disabled={
              busy || !running || !account.enabled || !account.tokenPresent
            }
            title={actionTitle}
            onClick={() =>
              void runAction('restart', () => restartOpenP2pAccount(account.id))
            }
          >
            {t('openp2p.account.actions.restart')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            startIcon={<FolderOpenOutlined />}
            disabled={busy}
            onClick={() =>
              void runAction('logs', () => openOpenP2pLogDir(account.id), false)
            }
          >
            {t('openp2p.account.actions.logs')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            startIcon={<VpnKeyOutlined />}
            disabled={busy}
            onClick={() => setTokenOpen(true)}
          >
            {t('openp2p.account.actions.token')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            startIcon={<KeyOutlined />}
            disabled={busy || !hasKeyNode}
            title={hasKeyNode ? undefined : t('openp2p.account.keyNoNode')}
            onClick={() => setKeyOpen(true)}
          >
            {t('openp2p.account.actions.key')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            color="error"
            startIcon={<DeleteOutlineRounded />}
            disabled={busy}
            onClick={() => setDeleteOpen(true)}
          >
            {t('openp2p.account.actions.delete')}
          </Button>
        </Stack>
      </Stack>

      {tokenOpen && (
        <OpenP2PTokenDialog
          accountId={account.id}
          onClose={() => setTokenOpen(false)}
        />
      )}
      {keyOpen && (
        <OpenP2PKeyDialog account={account} onClose={() => setKeyOpen(false)} />
      )}

      <BaseDialog
        open={deleteOpen}
        title={t('openp2p.account.deleteConfirm.title')}
        okBtn={t('openp2p.account.deleteConfirm.confirm')}
        cancelBtn={t('shared.actions.cancel')}
        loading={busy}
        disableCancel={busy}
        onOk={handleDelete}
        onCancel={() => setDeleteOpen(false)}
        onClose={busy ? undefined : () => setDeleteOpen(false)}
      >
        <Typography variant="body2">
          {t('openp2p.account.deleteConfirm.message', {
            label: account.label,
          })}
        </Typography>
      </BaseDialog>
    </Paper>
  )
}
