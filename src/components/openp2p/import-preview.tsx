import {
  Alert,
  Box,
  Chip,
  Divider,
  Stack,
  Typography,
  type ChipProps,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import type {
  OpenP2pCredentialState,
  OpenP2pImportAccount,
  OpenP2pImportAction,
  OpenP2pImportPreview,
} from '@/services/cmds'
import type { TranslationKey } from '@/types/generated/i18n-keys'

import { translateOpenP2pIssue, translateOpenP2pWarning } from './errors'

const ACTION_LABEL_KEYS: Record<OpenP2pImportAction, TranslationKey> = {
  add: 'openp2p.import.actions.add',
  update: 'openp2p.import.actions.update',
  unchanged: 'openp2p.import.actions.unchanged',
  remove: 'openp2p.import.actions.remove',
}

const ACTION_COLORS: Record<OpenP2pImportAction, ChipProps['color']> = {
  add: 'success',
  update: 'info',
  unchanged: 'default',
  remove: 'error',
}

const CREDENTIAL_LABEL_KEYS: Record<OpenP2pCredentialState, TranslationKey> = {
  inline: 'openp2p.import.credential.inline',
  present: 'openp2p.import.credential.present',
  missing: 'openp2p.import.credential.missing',
}

const CREDENTIAL_COLORS: Record<OpenP2pCredentialState, ChipProps['color']> = {
  inline: 'info',
  present: 'success',
  missing: 'warning',
}

const CredentialChip = ({
  name,
  state,
}: {
  name: TranslationKey
  state: OpenP2pCredentialState
}) => {
  const { t } = useTranslation()
  return (
    <Chip
      size="small"
      variant="outlined"
      color={CREDENTIAL_COLORS[state]}
      label={`${t(name)}: ${t(CREDENTIAL_LABEL_KEYS[state])}`}
    />
  )
}

const PreviewAccount = ({ account }: { account: OpenP2pImportAccount }) => {
  const { t } = useTranslation()
  return (
    <Box
      sx={{
        py: 1,
        borderTop: 1,
        borderColor: 'divider',
        '&:first-of-type': { borderTop: 0, pt: 0 },
      }}
    >
      <Stack spacing={0.75}>
        <Stack
          direction="row"
          spacing={1}
          sx={{ alignItems: 'center', flexWrap: 'wrap' }}
        >
          <Chip
            size="small"
            variant="outlined"
            color={ACTION_COLORS[account.action]}
            label={t(ACTION_LABEL_KEYS[account.action])}
          />
          <Typography variant="body2" sx={{ fontWeight: 600 }}>
            {account.label}
          </Typography>
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ fontFamily: 'monospace' }}
          >
            {account.id}
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {t('openp2p.import.routes', { count: account.routeCount })}
          </Typography>
        </Stack>

        <Stack
          direction="row"
          spacing={0.5}
          useFlexGap
          sx={{ flexWrap: 'wrap' }}
        >
          <CredentialChip
            name="openp2p.import.credential.token"
            state={account.token}
          />
          <CredentialChip
            name="openp2p.import.credential.key"
            state={account.key}
          />
          {account.nodes.map((node) => (
            <Chip
              key={node}
              size="small"
              variant="outlined"
              label={t('openp2p.import.node', { name: node })}
              sx={{ fontFamily: 'monospace' }}
            />
          ))}
        </Stack>

        {account.nodes.length === 0 && (
          <Typography variant="caption" color="text.disabled">
            {t('openp2p.import.noNodes')}
          </Typography>
        )}

        {account.warnings.length > 0 && (
          <Stack
            direction="row"
            spacing={0.5}
            useFlexGap
            sx={{ flexWrap: 'wrap' }}
          >
            {account.warnings.map((warning) => (
              <Chip
                key={warning}
                size="small"
                color="warning"
                variant="outlined"
                label={translateOpenP2pWarning(warning, t)}
              />
            ))}
          </Stack>
        )}
      </Stack>
    </Box>
  )
}

export const OpenP2PImportPreviewPanel = ({
  preview,
}: {
  preview: OpenP2pImportPreview
}) => {
  const { t } = useTranslation()
  return (
    <>
      <Divider />
      <Box aria-live="polite">
        <Stack spacing={0.75}>
          <Typography variant="subtitle2">
            {t('openp2p.import.source')}
          </Typography>
          <Stack
            direction="row"
            spacing={0.5}
            useFlexGap
            sx={{ flexWrap: 'wrap' }}
          >
            <Chip
              size="small"
              variant="outlined"
              label={t(
                `openp2p.page.source.kind.${preview.source.kind}` as TranslationKey,
              )}
            />
            {preview.source.name && (
              <Chip
                size="small"
                variant="outlined"
                label={preview.source.name}
              />
            )}
            {preview.source.version && (
              <Chip
                size="small"
                variant="outlined"
                label={t('openp2p.import.version', {
                  version: preview.source.version,
                })}
              />
            )}
            <Chip
              size="small"
              variant="outlined"
              label={
                preview.source.containsMihomo
                  ? t('openp2p.import.containsMihomo')
                  : t('openp2p.import.mihomoOnly')
              }
            />
            <Chip
              size="small"
              variant="outlined"
              color={preview.source.selfContained ? 'success' : 'default'}
              label={
                preview.source.selfContained
                  ? t('openp2p.import.selfContained')
                  : t('openp2p.import.policyOnly')
              }
            />
          </Stack>
          <Typography variant="caption" color="text.secondary">
            {t('openp2p.import.mihomoHint')}
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {t('openp2p.import.deviceHint')}
          </Typography>
        </Stack>
      </Box>

      <Box>
        <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
          {t('openp2p.import.issues')}
        </Typography>
        {preview.issues.length === 0 ? (
          <Alert severity="success">{t('openp2p.import.noIssues')}</Alert>
        ) : (
          <Stack spacing={0.75}>
            {preview.issues.map((issue) => (
              <Alert
                key={`${issue.code}:${issue.path ?? ''}:${issue.message}`}
                severity={issue.severity}
              >
                <Typography variant="body2">
                  {translateOpenP2pIssue(issue.code, issue.message, t)}
                </Typography>
                {issue.path && (
                  <Typography
                    variant="caption"
                    sx={{ fontFamily: 'monospace' }}
                  >
                    {issue.path}
                  </Typography>
                )}
              </Alert>
            ))}
          </Stack>
        )}
      </Box>

      <Box>
        <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
          {t('openp2p.import.accounts')} · {preview.accounts.length}
        </Typography>
        {preview.accounts.length === 0 ? (
          <Typography variant="body2" color="text.disabled">
            {t('openp2p.import.noAccounts')}
          </Typography>
        ) : (
          <Box
            sx={{
              border: 1,
              borderColor: 'divider',
              borderRadius: 1,
              px: 1.25,
              py: 1,
            }}
          >
            {preview.accounts.map((account) => (
              <PreviewAccount key={account.id} account={account} />
            ))}
          </Box>
        )}
      </Box>

      {preview.accounts.some((account) => account.action === 'remove') && (
        <Alert severity="warning">{t('openp2p.import.removeWarning')}</Alert>
      )}
    </>
  )
}
