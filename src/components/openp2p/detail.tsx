import { Box, Typography } from '@mui/material'
import { useTranslation } from 'react-i18next'

import type { TranslationKey } from '@/types/generated/i18n-keys'

export const OpenP2PDetail = ({
  label,
  value,
  mono = true,
}: {
  label: TranslationKey
  value: string | number | null | undefined
  mono?: boolean
}) => {
  const { t } = useTranslation()
  const text = value === null || value === undefined ? '' : String(value)
  return (
    <Box sx={{ minWidth: 0 }}>
      <Typography variant="caption" color="text.secondary">
        {t(label)}
      </Typography>
      <Typography
        variant="body2"
        sx={{
          ...(mono ? { fontFamily: 'monospace' } : {}),
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
        title={text || undefined}
      >
        {text === '' ? '—' : text}
      </Typography>
    </Box>
  )
}
