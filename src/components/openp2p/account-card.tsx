import { FolderOpenOutlined } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import type { OpenP2pAccount } from '@/services/cmds'
import { openOpenP2pLogDir } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'

import { OpenP2PDetail } from './detail'
import { translateOpenP2pErrorText } from './errors'
import {
  OpenP2PKeyBadge,
  OpenP2PListenBadge,
  OpenP2PLoginBadge,
  OpenP2PPhaseBadge,
  OpenP2PTcpBadge,
  OpenP2PTokenBadge,
} from './status-badges'

const WARNING_PHASES = new Set(['degraded', 'backoff', 'tokenMissing'])

export const OpenP2PAccountCard = ({
  account,
}: {
  account: OpenP2pAccount
}) => {
  const { t } = useTranslation()
  const hasKeyNode = account.keyRefs.length > 0

  const lastError = account.lastError
    ? translateOpenP2pErrorText(account.lastError, t)
    : null

  return (
    <Paper
      component="section"
      aria-label={`${account.label} (${account.id})`}
      variant="outlined"
      sx={{ p: 2, borderRadius: 2 }}
    >
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
              md: 'repeat(5, minmax(0, 1fr))',
            },
            gap: 1,
          }}
        >
          <OpenP2PDetail
            label="openp2p.account.gateway"
            value={
              account.gateway.host
                ? `${account.gateway.host}:${account.gateway.port}`
                : ''
            }
          />
          <OpenP2PDetail label="openp2p.account.node" value={account.node} />
          <OpenP2PDetail
            label="openp2p.account.publicPort"
            value={account.publicIpPort}
          />
          <OpenP2PDetail
            label="openp2p.account.interface"
            value={account.interface}
          />
          <OpenP2PDetail
            label="openp2p.account.routes"
            value={account.routeCount ?? ''}
            mono={false}
          />
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

        {lastError && (
          <Alert
            severity={WARNING_PHASES.has(account.phase) ? 'warning' : 'error'}
          >
            <Typography
              variant="body2"
              sx={{ whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}
            >
              {lastError}
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
                <Typography
                  variant="caption"
                  color="text.secondary"
                  noWrap
                  title={app.banner || undefined}
                  sx={{
                    fontFamily: 'monospace',
                    display: 'block',
                    minHeight: '1.2em',
                  }}
                >
                  {app.banner || ' '}
                </Typography>
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
            variant="outlined"
            startIcon={<FolderOpenOutlined />}
            onClick={() => {
              void openOpenP2pLogDir(account.id).catch(showNotice.error)
            }}
          >
            {t('openp2p.account.actions.logs')}
          </Button>
        </Stack>
      </Stack>
    </Paper>
  )
}
