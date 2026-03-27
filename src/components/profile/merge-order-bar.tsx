import { DndContext, closestCenter, DragEndEvent } from '@dnd-kit/core'
import {
  SortableContext,
  horizontalListSortingStrategy,
  useSortable,
  arrayMove,
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import ClearRounded from '@mui/icons-material/ClearRounded'
import WarningAmberRounded from '@mui/icons-material/WarningAmberRounded'
import { Badge, Chip, IconButton, Paper, Tooltip } from '@mui/material'
import { useTranslation } from 'react-i18next'

interface MergeOrderBarProps {
  mergedUids: string[]
  profiles: IProfileItem[]
  conflictCount: number
  onReorder: (newUids: string[]) => void
  onClear: () => void
  onShowConflicts: () => void
}

interface SortableChipProps {
  uid: string
  label: string
  isPrimary: boolean
}

function SortableChip({ uid, label, isPrimary }: SortableChipProps) {
  const { attributes, listeners, setNodeRef, transform, transition } =
    useSortable({ id: uid })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    cursor: 'grab',
  }

  return (
    <Chip
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...listeners}
      label={label}
      color={isPrimary ? 'primary' : 'default'}
      size="small"
      variant={isPrimary ? 'filled' : 'outlined'}
    />
  )
}

export function MergeOrderBar({
  mergedUids,
  profiles,
  conflictCount,
  onReorder,
  onClear,
  onShowConflicts,
}: MergeOrderBarProps) {
  const { t } = useTranslation()

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over && active.id !== over.id) {
      const oldIndex = mergedUids.indexOf(active.id as string)
      const newIndex = mergedUids.indexOf(over.id as string)
      onReorder(arrayMove(mergedUids, oldIndex, newIndex))
    }
  }

  const getName = (uid: string): string =>
    profiles.find((p) => p.uid === uid)?.name ?? uid

  return (
    <Paper
      elevation={0}
      variant="outlined"
      sx={{
        display: 'flex',
        alignItems: 'center',
        gap: 1,
        px: 1.5,
        py: 0.75,
        mb: 1,
        overflowX: 'auto',
        flexWrap: 'nowrap',
      }}
    >
      <DndContext collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
        <SortableContext
          items={mergedUids}
          strategy={horizontalListSortingStrategy}
        >
          {mergedUids.map((uid, i) => (
            <SortableChip
              key={uid}
              uid={uid}
              label={getName(uid)}
              isPrimary={i === 0}
            />
          ))}
        </SortableContext>
      </DndContext>

      <Tooltip title={t('profiles.merge.conflicts.badge')}>
        <span>
          <IconButton
            size="small"
            onClick={onShowConflicts}
            color={conflictCount > 0 ? 'warning' : 'default'}
            disabled={conflictCount === 0}
          >
            <Badge badgeContent={conflictCount} color="warning" max={99}>
              <WarningAmberRounded fontSize="small" />
            </Badge>
          </IconButton>
        </span>
      </Tooltip>

      <Tooltip title={t('profiles.merge.clear')}>
        <IconButton size="small" onClick={onClear}>
          <ClearRounded fontSize="small" />
        </IconButton>
      </Tooltip>
    </Paper>
  )
}
