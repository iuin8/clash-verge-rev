import { Chip, type ChipProps } from '@mui/material'
import { useTranslation } from 'react-i18next'

import type { OpenP2PCheckState, OpenP2PPhase } from '@/services/bindings'
import type { TranslationKey } from '@/types/generated/i18n-keys'

const PHASE_LABEL_KEYS: Record<OpenP2PPhase, TranslationKey> = {
  stopped: 'openp2p.account.phase.stopped',
  starting: 'openp2p.account.phase.starting',
  loginPending: 'openp2p.account.phase.loginPending',
  online: 'openp2p.account.phase.online',
  degraded: 'openp2p.account.phase.degraded',
  backoff: 'openp2p.account.phase.backoff',
  authFailed: 'openp2p.account.phase.authFailed',
  configFailed: 'openp2p.account.phase.configFailed',
  tokenMissing: 'openp2p.account.phase.tokenMissing',
  stopping: 'openp2p.account.phase.stopping',
  failed: 'openp2p.account.phase.failed',
}

const PHASE_COLORS: Record<OpenP2PPhase, ChipProps['color']> = {
  stopped: 'default',
  starting: 'info',
  loginPending: 'info',
  online: 'success',
  degraded: 'warning',
  backoff: 'warning',
  authFailed: 'error',
  configFailed: 'error',
  tokenMissing: 'warning',
  stopping: 'info',
  failed: 'error',
}

export const OpenP2PPhaseBadge = ({ phase }: { phase: OpenP2PPhase }) => {
  const { t } = useTranslation()
  return (
    <Chip
      size="small"
      color={PHASE_COLORS[phase]}
      label={t(PHASE_LABEL_KEYS[phase])}
    />
  )
}

const LOGIN_LABEL_KEYS: Record<OpenP2PCheckState, TranslationKey> = {
  unknown: 'openp2p.account.checks.loginUnknown',
  pending: 'openp2p.account.checks.loginPending',
  ok: 'openp2p.account.checks.loginOk',
  failed: 'openp2p.account.checks.loginFailed',
}

const LOGIN_COLORS: Record<OpenP2PCheckState, ChipProps['color']> = {
  unknown: 'default',
  pending: 'info',
  ok: 'success',
  failed: 'error',
}

export const OpenP2PLoginBadge = ({
  state,
  detail,
}: {
  state: OpenP2PCheckState
  detail?: string | null
}) => {
  const { t } = useTranslation()
  return (
    <Chip
      size="small"
      variant="outlined"
      color={LOGIN_COLORS[state]}
      label={t(LOGIN_LABEL_KEYS[state])}
      title={detail ?? undefined}
    />
  )
}

export const OpenP2PListenBadge = ({ listening }: { listening: boolean }) => {
  const { t } = useTranslation()
  return (
    <Chip
      size="small"
      variant="outlined"
      color={listening ? 'success' : 'default'}
      label={
        listening
          ? t('openp2p.account.checks.listen')
          : t('openp2p.account.checks.notListening')
      }
    />
  )
}

export const OpenP2PTcpBadge = ({
  reachable,
}: {
  reachable: boolean | null | undefined
}) => {
  const { t } = useTranslation()
  if (reachable === undefined || reachable === null) {
    return (
      <Chip
        size="small"
        variant="outlined"
        label={t('openp2p.account.checks.tcpPending')}
      />
    )
  }
  return (
    <Chip
      size="small"
      variant="outlined"
      color={reachable ? 'success' : 'error'}
      label={
        reachable
          ? t('openp2p.account.checks.tcpReachable')
          : t('openp2p.account.checks.tcpUnreachable')
      }
    />
  )
}

export const OpenP2PTokenBadge = ({ present }: { present: boolean }) => {
  const { t } = useTranslation()
  return (
    <Chip
      size="small"
      variant="outlined"
      color={present ? 'success' : 'default'}
      label={
        present
          ? t('openp2p.account.tokenSaved')
          : t('openp2p.account.tokenMissing')
      }
    />
  )
}

export const OpenP2PKeyBadge = ({
  present,
  hasNode,
}: {
  present: boolean
  hasNode: boolean
}) => {
  const { t } = useTranslation()
  if (!hasNode) {
    return (
      <Chip
        size="small"
        variant="outlined"
        label={t('openp2p.account.keyNoNode')}
      />
    )
  }
  return (
    <Chip
      size="small"
      variant="outlined"
      color={present ? 'success' : 'default'}
      label={
        present
          ? t('openp2p.account.keySaved')
          : t('openp2p.account.keyMissing')
      }
    />
  )
}
