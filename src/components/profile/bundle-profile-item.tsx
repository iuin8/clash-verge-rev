import { RefreshRounded } from '@mui/icons-material'
import { Box, Chip, IconButton, Typography } from '@mui/material'
import { memo } from 'react'
import { useTranslation } from 'react-i18next'

import type { BundleView, ConflictEntry } from '@/services/bindings'

import { isDefaultBundleName } from './bundle-constants'
import { bundleRefreshTime } from './bundle-time'
import { ProfileItem, type ContextMenuItem } from './profile-item'

export interface BundleProfileItemProps {
  id: string
  index: number
  selected: boolean
  activating: boolean
  itemData: IProfileItem
  mutateProfiles: () => Promise<void>
  onSelect: (force: boolean) => void
  batchMode?: boolean
  view: BundleView | null
  conflicts: ConflictEntry[]
  busy: boolean
  onEditMembers: () => void
  onDetails: () => void
  onRefresh: () => void
  onDelete: () => void
}

const boxStyle = {
  height: 26,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
}

// Badge chip + inline refresh icon widths in the fixed title row.
const TITLE_ADORNMENT_RESERVE = 92

/** FORK: Profile Bundle card; keeps bundle UI out of the upstream profile card. */
const BundleProfileItemBase = (props: BundleProfileItemProps) => {
  const {
    id,
    index,
    selected,
    activating,
    itemData,
    mutateProfiles,
    onSelect,
    batchMode,
    view,
    conflicts,
    busy,
    onEditMembers,
    onDetails,
    onRefresh,
    onDelete,
  } = props

  const { t } = useTranslation()
  const storedName = itemData.name
  const displayName = isDefaultBundleName(storedName)
    ? t('profiles.page.bundle.defaultName')
    : storedName
  const refreshed = bundleRefreshTime(view?.generated_at)

  const menu: ContextMenuItem[] = [
    {
      label: 'profiles.components.menu.select',
      handler: () => onSelect(true),
      disabled: false,
    },
    {
      label: 'profiles.page.bundle.actions.editMembers',
      handler: onEditMembers,
      disabled: activating || !view,
    },
    {
      label: 'profiles.page.bundle.actions.conflicts',
      handler: onDetails,
      disabled: !view,
    },
    {
      label: 'profiles.page.bundle.actions.refresh',
      handler: onRefresh,
      disabled: activating || busy,
    },
    {
      label: 'profiles.page.bundle.actions.delete',
      handler: onDelete,
      disabled: activating,
      danger: true,
    },
  ]

  const titleAdornment = (
    <>
      <Chip
        size="small"
        color="primary"
        variant="outlined"
        label={t('profiles.components.bundle.badge')}
        sx={{ ml: 0.5, height: 20, flexShrink: 0 }}
      />
      <IconButton
        aria-label={t('profiles.page.bundle.actions.refresh')}
        title={t('profiles.page.bundle.actions.refresh')}
        size="small"
        color="inherit"
        disabled={activating || busy}
        sx={{ ml: 'auto', p: '3px', flexShrink: 0 }}
        onClick={(event) => {
          event.stopPropagation()
          onRefresh()
        }}
      >
        <RefreshRounded fontSize="small" />
      </IconButton>
    </>
  )

  const details = (
    <>
      <Box sx={boxStyle}>
        <Typography noWrap sx={{ fontSize: 14 }}>
          {t('profiles.components.bundle.members', {
            count: view?.members.length ?? 0,
          })}
        </Typography>
        <Typography
          noWrap
          sx={{ fontSize: 14, ml: 'auto', maxWidth: '60%' }}
          title={view?.members[0]?.name}
        >
          {t('profiles.components.bundle.primary', {
            name: view?.members[0]?.name ?? '-',
          })}
        </Typography>
      </Box>
      <Box sx={{ ...boxStyle, fontSize: 12, gap: 1 }}>
        <span style={{ whiteSpace: 'nowrap' }}>
          {t('profiles.components.bundle.conflicts', {
            count: conflicts.length,
          })}
        </span>
        <Box
          component="span"
          sx={{
            ml: 'auto',
            minWidth: 0,
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'flex-end',
            gap: 0.75,
            overflow: 'hidden',
          }}
        >
          {view?.stale && (
            <Box
              component="span"
              sx={{
                color: 'warning.main',
                whiteSpace: 'nowrap',
                flexShrink: 0,
              }}
              title={t('profiles.components.bundle.stale')}
            >
              {t('profiles.components.bundle.stale')}
            </Box>
          )}
          <Box
            component="span"
            sx={{
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
            title={refreshed.absolute ?? undefined}
          >
            {refreshed.relative
              ? t('profiles.components.bundle.refreshed', {
                  time: refreshed.relative,
                })
              : t('profiles.components.bundle.refreshedUnknown')}
          </Box>
        </Box>
      </Box>
    </>
  )

  return (
    <ProfileItem
      id={id}
      index={index}
      selected={selected}
      activating={activating}
      itemData={itemData}
      mutateProfiles={mutateProfiles}
      onSelect={onSelect}
      onEdit={() => {}}
      onDelete={onDelete}
      batchMode={batchMode}
      selectable={false}
      titleOverride={displayName ?? undefined}
      titleAdornment={titleAdornment}
      titleAdornmentReserve={TITLE_ADORNMENT_RESERVE}
      details={details}
      menu={menu}
      timerUpdateRevision={0}
      completedUpdateRevision={0}
    />
  )
}

export const BundleProfileItem = memo(BundleProfileItemBase)
