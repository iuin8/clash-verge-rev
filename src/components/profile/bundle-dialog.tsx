import {
  DeleteOutlineRounded,
  EditRounded,
  RefreshRounded,
  WarningAmberRounded,
} from '@mui/icons-material'
import {
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  List,
  ListItem,
  Typography,
} from '@mui/material'
import dayjs from 'dayjs'
import { useTranslation } from 'react-i18next'

import type {
  BundleMemberView,
  BundleView,
  ConflictEntry,
} from '@/services/bindings'

export interface BundleDraftMember {
  uid: string
  name: string
}

function conflictMessage(
  t: ReturnType<typeof useTranslation>['t'],
  conflict: ConflictEntry,
) {
  const params = {
    field: conflict.field,
    name: conflict.name,
    source: conflict.source,
  }
  return t(`profiles.page.bundle.conflict.${conflict.code}`, {
    ...params,
    defaultValue: t('profiles.page.bundle.conflict.unknown', params),
  })
}

interface DetailsProps {
  open: boolean
  view: BundleView | null
  conflicts: ConflictEntry[]
  busy: boolean
  onClose: () => void
  onEditMembers: () => void
  onRefresh: () => void
  onDelete: () => void
}

export function BundleDetailsDialog({
  open,
  view,
  conflicts,
  busy,
  onClose,
  onEditMembers,
  onRefresh,
  onDelete,
}: DetailsProps) {
  const { t } = useTranslation()

  if (!view) return null

  const generatedAt =
    view.generated_at > 0
      ? dayjs(view.generated_at * 1000).format('YYYY-MM-DD HH:mm')
      : '-'

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{t('profiles.page.bundle.details.title')}</DialogTitle>
      <DialogContent>
        <Box
          sx={{
            display: 'flex',
            alignItems: 'center',
            gap: 1,
            flexWrap: 'wrap',
            mb: 1,
          }}
        >
          <Typography variant="h6" component="span">
            {view.name}
          </Typography>
          <Chip
            size="small"
            label={`${t('profiles.page.bundle.details.members')}: ${view.members.length}`}
          />
          {view.stale && (
            <Chip
              size="small"
              color="warning"
              icon={<WarningAmberRounded />}
              label={t('profiles.components.bundle.stale')}
            />
          )}
        </Box>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: 'block', mb: 1 }}
        >
          {t('profiles.page.bundle.details.generatedAt', {
            time: generatedAt,
          })}
        </Typography>
        {view.stale && (
          <Typography variant="body2" color="warning.main" sx={{ mb: 1 }}>
            {t('profiles.page.bundle.details.stale')}
          </Typography>
        )}

        <Typography variant="subtitle2" sx={{ mt: 1, mb: 0.5 }}>
          {t('profiles.page.bundle.details.members')}
        </Typography>
        <List dense disablePadding>
          {view.members.map((member, index) => (
            <ListItem
              key={member.uid}
              disableGutters
              sx={{ display: 'flex', alignItems: 'center', gap: 1, py: 0.25 }}
            >
              <Typography
                variant="body2"
                sx={{ minWidth: 0, flex: 1, wordBreak: 'break-all' }}
              >
                {index + 1}. {member.name}
              </Typography>
              {index === 0 && (
                <Chip
                  size="small"
                  color="primary"
                  variant="outlined"
                  label={t('profiles.page.bundle.details.primary')}
                />
              )}
              {member.kind === 'missing' && (
                <Chip
                  size="small"
                  color="error"
                  label={t('profiles.page.bundle.details.missing')}
                />
              )}
            </ListItem>
          ))}
        </List>

        <Divider sx={{ my: 1 }} />
        <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
          {t('profiles.page.bundle.details.conflicts')}
        </Typography>
        {conflicts.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            {t('profiles.page.bundle.details.noConflicts')}
          </Typography>
        ) : (
          <Box sx={{ maxHeight: 180, overflow: 'auto' }}>
            {conflicts.map((conflict) => (
              <Typography
                key={`${conflict.code}-${conflict.field}-${conflict.name}-${conflict.source}`}
                variant="caption"
                sx={{ display: 'block' }}
              >
                {conflictMessage(t, conflict)}
              </Typography>
            ))}
          </Box>
        )}
      </DialogContent>
      <DialogActions>
        <Button
          startIcon={<EditRounded />}
          onClick={onEditMembers}
          disabled={busy}
        >
          {t('profiles.page.bundle.actions.editMembers')}
        </Button>
        {view.stale && (
          <Button
            startIcon={<RefreshRounded />}
            onClick={onRefresh}
            disabled={busy}
          >
            {t('profiles.page.bundle.actions.refresh')}
          </Button>
        )}
        <Button
          color="error"
          startIcon={<DeleteOutlineRounded />}
          onClick={onDelete}
          disabled={busy}
        >
          {t('profiles.page.bundle.actions.delete')}
        </Button>
        <Button variant="contained" onClick={onClose} disabled={busy}>
          {t('shared.actions.close')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}

interface ConfirmProps {
  open: boolean
  mode: 'create' | 'overwrite'
  members: BundleDraftMember[]
  currentMembers: BundleMemberView[]
  busy: boolean
  onCancel: () => void
  onConfirm: () => void
}

type MemberDiff = 'added' | 'moved' | 'removed' | 'unchanged'

function diffMembers(
  members: BundleDraftMember[],
  currentMembers: BundleMemberView[],
) {
  const currentIndex = new Map(
    currentMembers.map((member, index) => [member.uid, index]),
  )
  const nextUids = new Set(members.map((member) => member.uid))
  const rows: { member: BundleDraftMember; status: MemberDiff }[] = members.map(
    (member, index) => {
      const previous = currentIndex.get(member.uid)
      if (previous === undefined) return { member, status: 'added' }
      return {
        member,
        status: previous === index ? 'unchanged' : 'moved',
      }
    },
  )
  const removed = currentMembers
    .filter((member) => !nextUids.has(member.uid))
    .map((member) => ({ member, status: 'removed' as const }))
  return [...rows, ...removed]
}

const DIFF_LABEL_KEYS = {
  added: 'profiles.page.bundle.diff.added',
  moved: 'profiles.page.bundle.diff.moved',
  removed: 'profiles.page.bundle.diff.removed',
  unchanged: 'profiles.page.bundle.diff.unchanged',
} as const

const DIFF_COLORS = {
  added: 'success.main',
  moved: 'warning.main',
  removed: 'error.main',
  unchanged: 'text.secondary',
} as const

export function BundleConfirmDialog({
  open,
  mode,
  members,
  currentMembers,
  busy,
  onCancel,
  onConfirm,
}: ConfirmProps) {
  const { t } = useTranslation()

  if (mode === 'overwrite') {
    const rows = diffMembers(members, currentMembers)
    return (
      <Dialog open={open} onClose={onCancel} maxWidth="sm" fullWidth>
        <DialogTitle>
          {t('profiles.page.bundle.confirm.overwrite.title')}
        </DialogTitle>
        <DialogContent>
          <Typography variant="body2" sx={{ mb: 1 }}>
            {t('profiles.page.bundle.confirm.overwrite.message')}
          </Typography>
          <List dense disablePadding>
            {rows.map(({ member, status }, index) => (
              <ListItem
                key={`${member.uid}-${status}`}
                disableGutters
                sx={{ display: 'flex', alignItems: 'center', gap: 1, py: 0.25 }}
              >
                <Typography
                  variant="body2"
                  sx={{ minWidth: 0, flex: 1, wordBreak: 'break-all' }}
                >
                  {status === 'removed'
                    ? member.name
                    : `${index + 1}. ${member.name}`}
                </Typography>
                <Typography
                  variant="caption"
                  sx={{ color: DIFF_COLORS[status] }}
                >
                  {t(DIFF_LABEL_KEYS[status])}
                </Typography>
              </ListItem>
            ))}
          </List>
        </DialogContent>
        <DialogActions>
          <Button onClick={onCancel} disabled={busy}>
            {t('shared.actions.cancel')}
          </Button>
          <Button variant="contained" onClick={onConfirm} loading={busy}>
            {t('profiles.page.bundle.batch.update')}
          </Button>
        </DialogActions>
      </Dialog>
    )
  }

  return (
    <Dialog open={open} onClose={onCancel} maxWidth="sm" fullWidth>
      <DialogTitle>
        {t('profiles.page.bundle.confirm.create.title')}
      </DialogTitle>
      <DialogContent>
        <Typography variant="body2" sx={{ mb: 1 }}>
          {t('profiles.page.bundle.confirm.create.message')}
        </Typography>
        <List dense disablePadding>
          {members.map((member, index) => (
            <ListItem
              key={member.uid}
              disableGutters
              sx={{ display: 'flex', alignItems: 'center', gap: 1, py: 0.25 }}
            >
              <Typography
                variant="body2"
                sx={{ minWidth: 0, flex: 1, wordBreak: 'break-all' }}
              >
                {index + 1}. {member.name}
              </Typography>
              {index === 0 && (
                <Chip
                  size="small"
                  color="primary"
                  variant="outlined"
                  label={t('profiles.page.bundle.details.primary')}
                />
              )}
            </ListItem>
          ))}
        </List>
      </DialogContent>
      <DialogActions>
        <Button onClick={onCancel} disabled={busy}>
          {t('shared.actions.cancel')}
        </Button>
        <Button variant="contained" onClick={onConfirm} loading={busy}>
          {t('profiles.page.bundle.batch.create')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
