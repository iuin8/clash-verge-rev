import {
  Alert,
  Box,
  Button,
  Chip,
  Divider,
  Stack,
  TextField,
  Typography,
  type ChipProps,
} from '@mui/material'
import { open } from '@tauri-apps/plugin-dialog'
import { readTextFile } from '@tauri-apps/plugin-fs'
import { useCallback, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog } from '@/components/base'
import type { OpenP2PPreview, OpenP2PPreviewAction } from '@/services/bindings'
import { applyOpenP2pConfig, previewOpenP2pConfig } from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'
import type { TranslationKey } from '@/types/generated/i18n-keys'

import { OpenP2PArtifactLists } from './artifact-lists'
import {
  openP2pFailureMessage,
  translateOpenP2pIssue,
  translateOpenP2pWarning,
} from './errors'

const MAX_YAML_BYTES = 1024 * 1024

const ACTION_LABEL_KEYS: Record<OpenP2PPreviewAction, TranslationKey> = {
  add: 'openp2p.import.actions.add',
  update: 'openp2p.import.actions.update',
  unchanged: 'openp2p.import.actions.unchanged',
  remove: 'openp2p.import.actions.remove',
}

const ACTION_COLORS: Record<OpenP2PPreviewAction, ChipProps['color']> = {
  add: 'success',
  update: 'info',
  unchanged: 'default',
  remove: 'error',
}

const fileNameOf = (path: string) => path.split(/[\\/]/).pop() || path

export const OpenP2PImportDialog = ({ onClose }: { onClose: () => void }) => {
  const { t } = useTranslation()
  const [yaml, setYaml] = useState('')
  const [fileName, setFileName] = useState('')
  const [preview, setPreview] = useState<OpenP2PPreview | null>(null)
  const [previewing, setPreviewing] = useState(false)
  const [applying, setApplying] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirmRemoveOpen, setConfirmRemoveOpen] = useState(false)
  const applyingRef = useRef(false)

  const previewConfig = useCallback(
    async (source: string) => {
      if (!source.trim()) {
        setPreview(null)
        setError(t('openp2p.import.emptyInput'))
        return
      }
      setPreviewing(true)
      setError(null)
      try {
        setPreview(await previewOpenP2pConfig(source))
      } catch (cause) {
        setPreview(null)
        setError(
          openP2pFailureMessage(cause, t('openp2p.import.previewFailed'), t),
        )
      } finally {
        setPreviewing(false)
      }
    },
    [t],
  )

  const chooseFile = async () => {
    setError(null)
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: t('openp2p.import.file'),
        filters: [{ name: 'YAML', extensions: ['yaml', 'yml'] }],
      })
      if (typeof selected !== 'string') return
      const content = await readTextFile(selected)
      if (content.length > MAX_YAML_BYTES) {
        setError(t('openp2p.import.fileTooLarge'))
        return
      }
      setFileName(fileNameOf(selected))
      setYaml(content)
      setPreview(null)
      await previewConfig(content)
    } catch (cause) {
      setError(
        openP2pFailureMessage(cause, t('openp2p.import.fileReadFailed'), t),
      )
    }
  }

  const apply = async () => {
    if (!preview?.canApply || applyingRef.current) return
    applyingRef.current = true
    setApplying(true)
    setError(null)
    try {
      const result = await mutate(() => applyOpenP2pConfig(yaml), {
        id: 'openp2p:apply',
        revalidate: [['getOpenP2pState'], ['getProfiles']],
        errorNotice: false,
      })
      if (!result.ok) {
        setConfirmRemoveOpen(false)
        setError(t('openp2p.import.applyFailed'))
        return
      }
      setConfirmRemoveOpen(false)
      onClose()
      showNotice.success(t('openp2p.import.applied'))
    } catch (cause) {
      setConfirmRemoveOpen(false)
      setError(openP2pFailureMessage(cause, t('openp2p.import.applyFailed'), t))
    } finally {
      applyingRef.current = false
      setApplying(false)
    }
  }

  const requestApply = () => {
    if (!preview?.canApply) return
    if (preview.accounts.some((account) => account.action === 'remove')) {
      setConfirmRemoveOpen(true)
      return
    }
    void apply()
  }

  const hasRemovals =
    preview?.accounts.some((account) => account.action === 'remove') ?? false

  return (
    <>
      <BaseDialog
        open
        title={t('openp2p.import.title')}
        okBtn={
          applying ? t('openp2p.import.applying') : t('openp2p.import.apply')
        }
        cancelBtn={t('shared.actions.cancel')}
        disableOk={!preview?.canApply || applying || previewing}
        disableCancel={applying}
        loading={applying}
        contentSx={{ minWidth: { sm: 520 } }}
        onOk={requestApply}
        onCancel={onClose}
        onClose={applying ? undefined : onClose}
      >
        <Stack spacing={1.5}>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Button
              variant="outlined"
              size="small"
              onClick={chooseFile}
              disabled={applying || previewing}
            >
              {t('openp2p.import.file')}
            </Button>
            {fileName && (
              <Typography
                variant="body2"
                color="text.secondary"
                noWrap
                sx={{ minWidth: 0 }}
                title={fileName}
              >
                {fileName}
              </Typography>
            )}
          </Stack>

          <TextField
            multiline
            fullWidth
            minRows={8}
            maxRows={16}
            label={t('openp2p.import.paste')}
            placeholder={t('openp2p.import.placeholder')}
            value={yaml}
            disabled={applying || previewing}
            onChange={(event) => {
              setYaml(event.target.value)
              setFileName('')
              setPreview(null)
              setError(null)
            }}
            slotProps={{
              inputLabel: { shrink: true },
              htmlInput: { spellCheck: false },
            }}
          />

          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Button
              variant="contained"
              size="small"
              loading={previewing}
              disabled={applying || previewing || !yaml.trim()}
              onClick={() => void previewConfig(yaml)}
            >
              {t('openp2p.import.preview')}
            </Button>
            {preview && (
              <Typography
                variant="body2"
                color={preview.canApply ? 'success.main' : 'error.main'}
              >
                {preview.canApply
                  ? t('openp2p.import.canApply')
                  : t('openp2p.import.cannotApply')}
              </Typography>
            )}
          </Stack>

          {error && <Alert severity="error">{error}</Alert>}

          {preview && (
            <>
              <Divider />
              <Typography variant="subtitle2">
                {t('openp2p.import.issues')}
              </Typography>
              {preview.issues.length === 0 ? (
                <Alert severity="success">{t('openp2p.import.noIssues')}</Alert>
              ) : (
                <Stack spacing={0.75}>
                  {preview.issues.map((issue) => (
                    <Alert
                      key={`${issue.code}:${issue.path}:${issue.message}`}
                      severity={issue.severity}
                    >
                      <Typography variant="body2">
                        {translateOpenP2pIssue(issue.code, issue.message, t)}
                      </Typography>
                      {issue.path && (
                        <Typography
                          variant="caption"
                          sx={{ fontFamily: 'monospace' }}
                        >
                          {issue.path}
                        </Typography>
                      )}
                    </Alert>
                  ))}
                </Stack>
              )}

              <Typography variant="subtitle2">
                {t('openp2p.import.diff')}
              </Typography>
              {preview.accounts.length === 0 ? (
                <Typography variant="body2" color="text.disabled">
                  {t('openp2p.artifacts.none')}
                </Typography>
              ) : (
                <Stack spacing={0.75}>
                  {preview.accounts.map((account) => (
                    <Stack
                      key={account.id}
                      direction="row"
                      spacing={1}
                      sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                    >
                      <Chip
                        size="small"
                        variant="outlined"
                        color={ACTION_COLORS[account.action]}
                        label={t(ACTION_LABEL_KEYS[account.action])}
                      />
                      <Box sx={{ minWidth: 0 }}>
                        <Typography variant="body2" noWrap>
                          {account.label}
                        </Typography>
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ fontFamily: 'monospace' }}
                        >
                          {account.id}
                        </Typography>
                      </Box>
                      {account.warnings.map((warning) => (
                        <Chip
                          key={warning}
                          size="small"
                          color="warning"
                          variant="outlined"
                          label={translateOpenP2pWarning(warning, t)}
                        />
                      ))}
                    </Stack>
                  ))}
                </Stack>
              )}

              {hasRemovals && (
                <Alert severity="warning">
                  {t('openp2p.import.removeWarning')}
                </Alert>
              )}

              <Divider />
              <Typography variant="subtitle2">
                {t('openp2p.import.generated')}
              </Typography>
              <OpenP2PArtifactLists
                proxies={preview.proxies}
                groups={preview.groups}
                rules={preview.rules}
                dnsPolicy={preview.dnsPolicy}
              />
            </>
          )}
        </Stack>
      </BaseDialog>

      {confirmRemoveOpen && (
        <BaseDialog
          open
          title={t('openp2p.import.confirmRemoveTitle')}
          okBtn={t('openp2p.import.confirmApply')}
          cancelBtn={t('shared.actions.cancel')}
          loading={applying}
          disableCancel={applying}
          onOk={() => void apply()}
          onCancel={() => setConfirmRemoveOpen(false)}
          onClose={applying ? undefined : () => setConfirmRemoveOpen(false)}
        >
          <Typography variant="body2">
            {t('openp2p.import.confirmRemoveMessage')}
          </Typography>
        </BaseDialog>
      )}
    </>
  )
}
