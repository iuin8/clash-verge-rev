import { Alert, Stack, TextField, Typography } from '@mui/material'
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog } from '@/components/base'
import { saveOpenP2pToken } from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'

import { openP2pFailureMessage } from './errors'

const TOKEN_PATTERN = /^[1-9]\d*$/

export const OpenP2PTokenDialog = ({
  accountId,
  onClose,
}: {
  accountId: string
  onClose: () => void
}) => {
  const { t } = useTranslation()
  const [token, setToken] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const savingRef = useRef(false)

  const handleSave = async () => {
    if (savingRef.current) return
    const trimmed = token.trim()
    if (!TOKEN_PATTERN.test(trimmed)) {
      setError(t('openp2p.token.invalid'))
      return
    }
    setError(null)
    savingRef.current = true
    setSaving(true)
    try {
      await mutate(() => saveOpenP2pToken(accountId, trimmed), {
        id: `openp2p-token:${accountId}`,
        revalidate: [['getOpenP2pState']],
        errorNotice: false,
      })
      setToken('')
      onClose()
      showNotice.success(t('openp2p.token.saved'))
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
      title={t('openp2p.token.title')}
      okBtn={t('openp2p.token.save')}
      cancelBtn={t('shared.actions.cancel')}
      disableOk={saving || token.trim().length === 0}
      disableCancel={saving}
      loading={saving}
      onOk={handleSave}
      onCancel={onClose}
      onClose={saving ? undefined : onClose}
    >
      <Stack spacing={1.5} sx={{ minWidth: { sm: 420 } }}>
        <Typography variant="body2" color="text.secondary">
          {t('openp2p.token.hint')}
        </Typography>
        <TextField
          autoFocus
          fullWidth
          size="small"
          type="password"
          label={t('openp2p.token.label')}
          placeholder={t('openp2p.token.placeholder')}
          value={token}
          disabled={saving}
          onChange={(event) => {
            setToken(event.target.value)
            setError(null)
          }}
          slotProps={{
            htmlInput: {
              autoComplete: 'new-password',
              spellCheck: false,
              'aria-label': t('openp2p.token.label'),
            },
          }}
        />
        {error && <Alert severity="error">{error}</Alert>}
      </Stack>
    </BaseDialog>
  )
}
