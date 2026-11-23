import {
  CheckCircleOutlineRounded,
  ErrorOutlineRounded,
  InsertDriveFileOutlined,
  LanguageOutlined,
  WarningAmberRounded,
} from '@mui/icons-material'
import {
  Box,
  Chip,
  Paper,
  Stack,
  Typography,
  type ChipProps,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import type { OpenP2pSource, OpenP2pSourceStatus } from '@/services/cmds'
import type { TranslationKey } from '@/types/generated/i18n-keys'

import { OpenP2PDetail } from './detail'

const STATUS_LABEL_KEYS: Record<OpenP2pSourceStatus, TranslationKey> = {
  applied: 'openp2p.page.source.statusValue.applied',
  stale: 'openp2p.page.source.statusValue.stale',
  error: 'openp2p.page.source.statusValue.error',
}

const STATUS_COLORS: Record<OpenP2pSourceStatus, ChipProps['color']> = {
  applied: 'success',
  stale: 'warning',
  error: 'error',
}

const STATUS_ICONS: Record<
  OpenP2pSourceStatus,
  typeof CheckCircleOutlineRounded
> = {
  applied: CheckCircleOutlineRounded,
  stale: WarningAmberRounded,
  error: ErrorOutlineRounded,
}

const SourceIcon = ({ kind }: { kind: OpenP2pSource['kind'] }) => {
  if (kind === 'url') return <LanguageOutlined fontSize="small" />
  if (kind === 'local') return <InsertDriveFileOutlined fontSize="small" />
  return null
}

const formatAppliedAt = (value: string) => {
  const numeric = /^\d+$/.test(value) ? Number(value) : Number.NaN
  const date = Number.isFinite(numeric)
    ? new Date(numeric > 1e12 ? numeric : numeric * 1000)
    : new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

export const OpenP2PSourceCard = ({
  source,
  accountCount,
}: {
  source: OpenP2pSource | null
  accountCount: number
}) => {
  const { t } = useTranslation()
  const status = source?.status
  const StatusIcon = status ? STATUS_ICONS[status] : null

  return (
    <Paper
      component="section"
      aria-labelledby="openp2p-source-title"
      variant="outlined"
      sx={{ p: 2, borderRadius: 2 }}
    >
      <Stack spacing={1.25}>
        <Stack
          direction="row"
          spacing={1}
          sx={{
            alignItems: 'center',
            justifyContent: 'space-between',
            flexWrap: 'wrap',
          }}
        >
          <Stack
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', minWidth: 0 }}
          >
            {source && (
              <Box sx={{ display: 'flex', color: 'text.secondary' }}>
                <SourceIcon kind={source.kind} />
              </Box>
            )}
            <Box sx={{ minWidth: 0 }}>
              <Typography
                id="openp2p-source-title"
                variant="subtitle1"
                sx={{ fontWeight: 600 }}
              >
                {t('openp2p.page.source.title')}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {source
                  ? (source.name ??
                    t(
                      `openp2p.page.source.kind.${source.kind}` as TranslationKey,
                    ))
                  : t('openp2p.page.source.none')}
              </Typography>
            </Box>
          </Stack>
          {source && (
            <Chip
              size="small"
              variant="outlined"
              color={status ? STATUS_COLORS[status] : 'default'}
              icon={StatusIcon ? <StatusIcon /> : undefined}
              label={
                status
                  ? t(STATUS_LABEL_KEYS[status])
                  : t(
                      `openp2p.page.source.kind.${source.kind}` as TranslationKey,
                    )
              }
            />
          )}
        </Stack>

        {source ? (
          <>
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
              {source.version && (
                <OpenP2PDetail
                  label="openp2p.page.source.version"
                  value={source.version}
                />
              )}
              {source.appliedAt && (
                <OpenP2PDetail
                  label="openp2p.page.source.appliedAt"
                  value={formatAppliedAt(source.appliedAt)}
                  mono={false}
                />
              )}
              <OpenP2PDetail
                label="openp2p.page.source.accountCount"
                value={accountCount}
                mono={false}
              />
            </Box>
            {(source.containsMihomo !== undefined ||
              source.selfContained !== undefined) && (
              <Stack
                direction="row"
                spacing={0.5}
                useFlexGap
                sx={{ flexWrap: 'wrap' }}
              >
                {source.containsMihomo !== undefined && (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={
                      source.containsMihomo
                        ? t('openp2p.page.source.containsMihomo')
                        : t('openp2p.page.source.mihomoOnly')
                    }
                  />
                )}
                {source.selfContained !== undefined && (
                  <Chip
                    size="small"
                    variant="outlined"
                    color={source.selfContained ? 'success' : 'default'}
                    label={
                      source.selfContained
                        ? t('openp2p.page.source.selfContained')
                        : t('openp2p.page.source.policyOnly')
                    }
                  />
                )}
              </Stack>
            )}
          </>
        ) : (
          <Typography variant="body2" color="text.secondary">
            {t('openp2p.page.source.noneHint')}
          </Typography>
        )}
      </Stack>
    </Paper>
  )
}
