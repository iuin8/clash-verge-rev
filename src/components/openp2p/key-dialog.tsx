import {
  Alert,
  Box,
  Button,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { open } from '@tauri-apps/plugin-dialog'
import { readTextFile } from '@tauri-apps/plugin-fs'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog } from '@/components/base'
import type { OpenP2PAccount_Serialize as OpenP2PAccount } from '@/services/bindings'
import { saveOpenP2pKey } from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'

import { openP2pFailureMessage } from './errors'

const MAX_PRIVATE_KEY_BYTES = 64 * 1024

const fileNameOf = (path: string) => path.split(/[\\/]/).pop() || path

export const OpenP2PKeyDialog = ({
  account,
  onClose,
}: {
  account: OpenP2PAccount
  onClose: () => void
}) => {
  const { t } = useTranslation()
  const refs = [...new Set(account.keyRefs)]
  const [selectedRef, setSelectedRef] = useState(refs[0] ?? '')
  const [fileName, setFileName] = useState('')
  const [fileSize, setFileSize] = useState<number | null>(null)
  const [selecting, setSelecting] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const privateKeyRef = useRef<string | null>(null)
  const autoOpenedRef = useRef(false)
  const savingRef = useRef(false)

  const chooseFile = useCallback(async () => {
    setError(null)
    setSelecting(true)
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: t('openp2p.key.chooseFile'),
      })
      if (typeof selected !== 'string') return
      const content = await readTextFile(selected)
      if (content.length > MAX_PRIVATE_KEY_BYTES) {
        privateKeyRef.current = null
        setFileName('')
        setFileSize(null)
        setError(t('openp2p.key.fileTooLarge'))
        return
      }
      if (
        !content.includes('-----BEGIN') ||
        !content.includes('PRIVATE KEY-----')
      ) {
        privateKeyRef.current = null
        setFileName('')
        setFileSize(null)
        setError(t('openp2p.key.invalidFile'))
        return
      }
      privateKeyRef.current = content
      setFileName(fileNameOf(selected))
      setFileSize(content.length)
    } catch (cause) {
      setError(openP2pFailureMessage(cause, t('openp2p.key.readFailed'), t))
    } finally {
      setSelecting(false)
    }
  }, [t])

  useEffect(() => {
    if (autoOpenedRef.current) return
    autoOpenedRef.current = true
    if (refs.length === 1) {
      void chooseFile()
    }
  }, [chooseFile, refs.length])

  useEffect(
    () => () => {
      privateKeyRef.current = null
    },
    [],
  )

  const handleSave = async () => {
    if (savingRef.current) return
    const privateKey = privateKeyRef.current
    if (!privateKey) {
      setError(t('openp2p.key.noFile'))
      return
    }
    if (!selectedRef || !refs.includes(selectedRef)) {
      setError(t('openp2p.key.noSshNode'))
      return
    }
    setError(null)
    savingRef.current = true
    setSaving(true)
    try {
      await mutate(() => saveOpenP2pKey(account.id, selectedRef, privateKey), {
        id: `openp2p-key:${account.id}`,
        revalidate: [['getOpenP2pState']],
        errorNotice: false,
      })
      privateKeyRef.current = null
      setFileName('')
      setFileSize(null)
      onClose()
      showNotice.success(t('openp2p.key.saved'))
    } catch (cause) {
      setError(
        openP2pFailureMessage(
          cause,
          t('shared.feedback.errors.operationFailed'),
          t,
        ),
      )
    } finally {
      savingRef.current = false
      setSaving(false)
    }
  }

  return (
    <BaseDialog
      open
      title={t('openp2p.key.title')}
      okBtn={t('openp2p.key.save')}
      cancelBtn={t('shared.actions.cancel')}
      disableOk={saving || refs.length === 0 || !privateKeyRef.current}
      disableCancel={saving}
      loading={saving}
      onOk={handleSave}
      onCancel={onClose}
      onClose={saving ? undefined : onClose}
    >
      <Stack spacing={1.5} sx={{ minWidth: { sm: 520 } }}>
        <Typography variant="body2" color="text.secondary">
          {t('openp2p.key.hint')}
        </Typography>

        {refs.length === 0 ? (
          <Alert severity="info">{t('openp2p.key.noSshNode')}</Alert>
        ) : refs.length === 1 ? (
          <Box>
            <Typography variant="caption" color="text.secondary">
              {t('openp2p.key.ref')}
            </Typography>
            <Typography variant="body2" sx={{ fontFamily: 'monospace' }}>
              {refs[0]}
            </Typography>
          </Box>
        ) : (
          <TextField
            select
            size="small"
            label={t('openp2p.key.ref')}
            value={selectedRef}
            disabled={saving}
            onChange={(event) => {
              setSelectedRef(event.target.value)
              setError(null)
            }}
            fullWidth
          >
            {refs.map((ref) => (
              <MenuItem key={ref} value={ref}>
                {ref}
              </MenuItem>
            ))}
          </TextField>
        )}

        {refs.length > 0 && (
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Button
              variant="outlined"
              size="small"
              loading={selecting}
              disabled={saving || selecting}
              onClick={chooseFile}
            >
              {t('openp2p.key.chooseFile')}
            </Button>
            {fileName && (
              <Typography
                variant="body2"
                color="text.secondary"
                noWrap
                sx={{ minWidth: 0 }}
                title={fileName}
              >
                {t('openp2p.key.selected', {
                  name: fileName,
                  size: fileSize ?? 0,
                })}
              </Typography>
            )}
          </Stack>
        )}

        {error && <Alert severity="error">{error}</Alert>}
      </Stack>
    </BaseDialog>
  )
}
