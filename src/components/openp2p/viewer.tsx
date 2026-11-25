import { CloseRounded } from '@mui/icons-material'
import {
  Chip,
  Dialog,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Typography,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import { OpenP2PPanel } from './panel'

export const OpenP2PViewer = ({
  open,
  version,
  onClose,
}: {
  open: boolean
  version?: string | null
  onClose: () => void
}) => {
  const { t } = useTranslation()
  return (
    <Dialog
      open={open}
      onClose={onClose}
      fullWidth
      maxWidth="md"
      scroll="paper"
    >
      <DialogTitle sx={{ pr: 6 }}>
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
          <Typography variant="h6">{t('openp2p.page.title')}</Typography>
          {version && (
            <Chip
              size="small"
              variant="outlined"
              label={`v${version}`}
              sx={{ fontFamily: 'monospace' }}
            />
          )}
          <IconButton
            size="small"
            onClick={onClose}
            aria-label={t('shared.actions.close')}
            sx={{ position: 'absolute', right: 8, top: 8 }}
          >
            <CloseRounded fontSize="small" />
          </IconButton>
        </Stack>
      </DialogTitle>
      <DialogContent dividers sx={{ p: 0 }}>
        <OpenP2PPanel />
      </DialogContent>
    </Dialog>
  )
}
