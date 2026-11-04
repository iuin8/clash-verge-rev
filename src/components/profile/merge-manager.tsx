import { ArrowDownward, ArrowUpward, MergeType } from '@mui/icons-material'
import {
  Box,
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  IconButton,
  List,
  ListItem,
  Stack,
  Typography,
} from '@mui/material'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useProfiles } from '@/hooks/use-profiles'
import type { ConflictEntry } from '@/services/bindings'
import {
  clearMergedProfiles,
  getMergeConflicts,
  getMergedProfiles,
  setMergedProfiles,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'

export function MergeManager() {
  const { t } = useTranslation()
  const { profiles } = useProfiles()
  const [open, setOpen] = useState(false)
  const [selected, setSelected] = useState<string[]>([])
  const [conflicts, setConflicts] = useState<ConflictEntry[]>([])
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    try {
      setSelected(await getMergedProfiles())
      setConflicts(await getMergeConflicts())
    } catch (error) {
      showNotice.error(error)
    }
  }, [])

  useEffect(() => {
    if (open) void load()
  }, [load, open])

  const items = useMemo(
    () =>
      (profiles?.items ?? []).filter(
        (item): item is typeof item & { uid: string } =>
          Boolean(item?.uid) &&
          (item.type === 'local' || item.type === 'remote'),
      ),
    [profiles?.items],
  )

  const toggle = (uid: string) => {
    setSelected((current) =>
      current.includes(uid)
        ? current.filter((id) => id !== uid)
        : [...current, uid],
    )
  }

  const move = (uid: string, offset: -1 | 1) => {
    setSelected((current) => {
      const index = current.indexOf(uid)
      const target = index + offset
      if (index < 0 || target < 0 || target >= current.length) return current
      const next = [...current]
      ;[next[index], next[target]] = [next[target], next[index]]
      return next
    })
  }

  const save = async () => {
    if (selected.length < 2) return
    setSaving(true)
    try {
      await mutate(() => setMergedProfiles(selected), {
        id: 'set-merged-profiles',
        revalidate: [['getProfiles']],
        errorNotice: false,
      })
      const nextConflicts = await getMergeConflicts()
      setConflicts(nextConflicts)
      showNotice.success(t('profiles.page.merge.saved'))
      if (nextConflicts.length === 0) setOpen(false)
    } catch (error) {
      showNotice.error(error)
    } finally {
      setSaving(false)
    }
  }

  const clear = async () => {
    setSaving(true)
    try {
      await mutate(() => clearMergedProfiles(), {
        id: 'clear-merged-profiles',
        revalidate: [['getProfiles']],
        errorNotice: false,
      })
      setSelected([])
      setConflicts([])
    } catch (error) {
      showNotice.error(error)
    } finally {
      setSaving(false)
    }
  }

  return (
    <>
      <IconButton
        size="small"
        color="inherit"
        title={t('profiles.page.merge.title')}
        onClick={() => setOpen(true)}
      >
        <MergeType />
      </IconButton>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>{t('profiles.page.merge.title')}</DialogTitle>
        <DialogContent>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
            {t('profiles.page.merge.description')}
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {t('profiles.page.merge.selectHint')}
          </Typography>
          <List dense sx={{ mt: 1 }}>
            {items.length === 0 && (
              <Typography variant="body2" color="text.secondary">
                {t('profiles.page.merge.empty')}
              </Typography>
            )}
            {items.map((item) => {
              const order = selected.indexOf(item.uid)
              return (
                <ListItem
                  key={item.uid}
                  secondaryAction={
                    order >= 0 ? (
                      <Stack direction="row" spacing={0.25}>
                        <IconButton
                          size="small"
                          disabled={order === 0}
                          onClick={() => move(item.uid, -1)}
                        >
                          <ArrowUpward fontSize="small" />
                        </IconButton>
                        <IconButton
                          size="small"
                          disabled={order === selected.length - 1}
                          onClick={() => move(item.uid, 1)}
                        >
                          <ArrowDownward fontSize="small" />
                        </IconButton>
                      </Stack>
                    ) : null
                  }
                >
                  <Checkbox
                    edge="start"
                    checked={order >= 0}
                    onChange={() => toggle(item.uid)}
                  />
                  <Box sx={{ minWidth: 0 }}>
                    <Typography noWrap>
                      {order >= 0 ? `${order + 1}. ` : ''}
                      {item.name || item.uid}
                    </Typography>
                    <Typography variant="caption" color="text.secondary" noWrap>
                      {item.uid}
                    </Typography>
                  </Box>
                </ListItem>
              )
            })}
          </List>
          {conflicts.length > 0 && (
            <>
              <Divider sx={{ my: 1 }} />
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                {t('profiles.page.merge.conflicts')}
              </Typography>
              <Box sx={{ maxHeight: 180, overflow: 'auto' }}>
                {conflicts.map((conflict) => (
                  <Typography
                    key={`${conflict.field}-${conflict.name}-${conflict.source}-${conflict.reason}`}
                    variant="caption"
                    sx={{ display: 'block' }}
                  >
                    {conflict.field}: {conflict.name} — {conflict.reason}
                  </Typography>
                ))}
              </Box>
            </>
          )}
        </DialogContent>
        <DialogActions>
          <Button disabled={saving} onClick={clear}>
            {t('profiles.page.merge.clear')}
          </Button>
          <Button
            variant="contained"
            disabled={saving || selected.length < 2}
            onClick={() => void save()}
          >
            {t('profiles.page.merge.save')}
          </Button>
        </DialogActions>
      </Dialog>
    </>
  )
}
