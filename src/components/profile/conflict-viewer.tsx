import {
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Typography,
} from '@mui/material'
import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseEmpty } from '@/components/base'

interface Props {
  open: boolean
  conflicts: ConflictEntry[]
  onClose: () => void
}

export const ConflictViewer = (props: Props) => {
  const { open, conflicts, onClose } = props

  const { t } = useTranslation()

  return (
    <Dialog open={open} onClose={onClose}>
      <DialogTitle>{t('profiles.merge.conflicts.title')}</DialogTitle>

      <DialogContent
        sx={{
          width: 400,
          height: 300,
          overflowX: 'hidden',
          userSelect: 'text',
          pb: 1,
        }}
      >
        {conflicts.map((entry) => (
          <Fragment key={`${entry.field}-${entry.source}-${entry.name}`}>
            <Typography color="text.secondary" component="div">
              <Chip
                label="skip"
                size="small"
                variant="outlined"
                color="warning"
                sx={{ mr: 1 }}
              />
              {entry.field} &ldquo;{entry.name}&rdquo; ({entry.source}) &mdash;{' '}
              {entry.reason}
            </Typography>
            <Divider sx={{ my: 0.5 }} />
          </Fragment>
        ))}

        {conflicts.length === 0 && (
          <BaseEmpty text={t('profiles.merge.conflicts.empty')} />
        )}
      </DialogContent>

      <DialogActions>
        <Button onClick={onClose} variant="outlined">
          {t('shared.actions.close')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
