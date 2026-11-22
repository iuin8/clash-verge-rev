import { Box, Chip, Stack, Typography } from '@mui/material'
import { useTranslation } from 'react-i18next'

import type { TranslationKey } from '@/types/generated/i18n-keys'

interface ArtifactSectionProps {
  title: TranslationKey
  items: readonly string[]
  monospace?: boolean
}

const ArtifactSection = ({
  title,
  items,
  monospace = false,
}: ArtifactSectionProps) => {
  const { t } = useTranslation()
  return (
    <Box sx={{ minWidth: 0 }}>
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ textTransform: 'uppercase', letterSpacing: 0.5 }}
      >
        {t(title)} · {items.length}
      </Typography>
      {items.length === 0 ? (
        <Typography variant="body2" color="text.disabled">
          {t('openp2p.artifacts.none')}
        </Typography>
      ) : monospace ? (
        <Box
          component="ul"
          sx={{
            m: 0,
            mt: 0.5,
            pl: 2,
            maxHeight: 160,
            overflow: 'auto',
            fontFamily: 'monospace',
            fontSize: 12,
            wordBreak: 'break-all',
          }}
        >
          {items.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </Box>
      ) : (
        <Stack
          direction="row"
          useFlexGap
          sx={{ flexWrap: 'wrap', gap: 0.5, mt: 0.5 }}
        >
          {items.map((item) => (
            <Chip key={item} size="small" variant="outlined" label={item} />
          ))}
        </Stack>
      )}
    </Box>
  )
}

export const OpenP2PArtifactLists = ({
  proxies,
  groups,
  rules,
  dnsPolicy,
}: {
  proxies: readonly string[]
  groups: readonly string[]
  rules: readonly string[]
  dnsPolicy: readonly string[]
}) => (
  <Box
    sx={{
      display: 'grid',
      gridTemplateColumns: { xs: '1fr', md: '1fr 1fr' },
      gap: 1.5,
    }}
  >
    <ArtifactSection title="openp2p.artifacts.proxies" items={proxies} />
    <ArtifactSection title="openp2p.artifacts.groups" items={groups} />
    <ArtifactSection title="openp2p.artifacts.rules" items={rules} monospace />
    <ArtifactSection
      title="openp2p.artifacts.dns"
      items={dnsPolicy}
      monospace
    />
  </Box>
)
