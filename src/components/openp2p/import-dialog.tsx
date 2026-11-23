import {
  Alert,
  Button,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from '@mui/material'
import { open } from '@tauri-apps/plugin-dialog'
import { readTextFile } from '@tauri-apps/plugin-fs'
import { useCallback, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog } from '@/components/base'
import type {
  OpenP2pImportPreview,
  OpenP2pImportRequest,
} from '@/services/cmds'
import { applyOpenP2pImport, previewOpenP2pImport } from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'

import { openP2pFailureMessage } from './errors'
import { OpenP2PImportPreviewPanel } from './import-preview'

const MAX_YAML_BYTES = 1024 * 1024

const fileNameOf = (path: string) => path.split(/[\\/]/).pop() || path

const isValidImportUrl = (value: string) => {
  try {
    return new URL(value).protocol === 'https:'
  } catch {
    return false
  }
}

export const OpenP2PImportDialog = ({
  onClose,
  onApplied,
}: {
  onClose: () => void
  onApplied: () => void
}) => {
  const { t } = useTranslation()
  const [mode, setMode] = useState<'yaml' | 'url'>('yaml')
  const [yaml, setYaml] = useState('')
  const [url, setUrl] = useState('')
  const [fileName, setFileName] = useState('')
  const [preview, setPreview] = useState<OpenP2pImportPreview | null>(null)
  const [previewRequest, setPreviewRequest] =
    useState<OpenP2pImportRequest | null>(null)
  const [previewing, setPreviewing] = useState(false)
  const [applying, setApplying] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirmRemoveOpen, setConfirmRemoveOpen] = useState(false)
  const applyingRef = useRef(false)
  const previewSequenceRef = useRef(0)

  const resetPreview = () => {
    previewSequenceRef.current += 1
    setPreview(null)
    setPreviewRequest(null)
    setPreviewing(false)
    setError(null)
  }

  const previewSource = useCallback(
    async (request: OpenP2pImportRequest) => {
      const sequence = ++previewSequenceRef.current
      setPreviewing(true)
      setError(null)
      try {
        const result = await previewOpenP2pImport(request)
        if (sequence !== previewSequenceRef.current) return
        setPreview(result)
        setPreviewRequest(request)
      } catch (cause) {
        if (sequence !== previewSequenceRef.current) return
        setPreview(null)
        setPreviewRequest(null)
        setError(
          openP2pFailureMessage(cause, t('openp2p.import.previewFailed'), t),
        )
      } finally {
        if (sequence === previewSequenceRef.current) setPreviewing(false)
      }
    },
    [t],
  )

  const buildRequest = (): OpenP2pImportRequest | null => {
    if (mode === 'url') {
      const value = url.trim()
      if (!value) {
        setError(t('openp2p.import.urlRequired'))
        return null
      }
      if (!isValidImportUrl(value)) {
        setError(t('openp2p.import.invalidUrl'))
        return null
      }
      return { url: value }
    }
    const content = yaml.trim()
    if (!content) {
      setError(t('openp2p.import.emptyInput'))
      return null
    }
    return { yaml: content }
  }

  const previewCurrent = () => {
    const request = buildRequest()
    if (request) void previewSource(request)
  }

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
        resetPreview()
        setError(t('openp2p.import.fileTooLarge'))
        return
      }
      setMode('yaml')
      setFileName(fileNameOf(selected))
      setYaml(content)
      setPreview(null)
      setPreviewRequest(null)
      await previewSource({ yaml: content })
    } catch (cause) {
      setError(
        openP2pFailureMessage(cause, t('openp2p.import.fileReadFailed'), t),
      )
    }
  }

  const apply = async () => {
    if (!preview?.canApply || !previewRequest || applyingRef.current) return
    applyingRef.current = true
    setApplying(true)
    setError(null)
    try {
      const result = await mutate(() => applyOpenP2pImport(previewRequest), {
        id: 'openp2p:import',
        revalidate: [['getOpenP2pState'], ['getProfiles'], ['getBundleView']],
      })
      if (!result.ok) {
        setConfirmRemoveOpen(false)
        setError(t('openp2p.import.applyFailed'))
        return
      }
      setConfirmRemoveOpen(false)
      showNotice.success(t('openp2p.import.applied'))
      onApplied()
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
        contentSx={{ minWidth: { sm: 560 } }}
        onOk={requestApply}
        onCancel={onClose}
        onClose={applying ? undefined : onClose}
      >
        <Stack spacing={1.5}>
          <ToggleButtonGroup
            exclusive
            size="small"
            value={mode}
            aria-label={t('openp2p.import.mode.label')}
            onChange={(_event, next: 'yaml' | 'url' | null) => {
              if (!next) return
              setMode(next)
              resetPreview()
            }}
          >
            <ToggleButton value="yaml">
              {t('openp2p.import.mode.yaml')}
            </ToggleButton>
            <ToggleButton value="url">
              {t('openp2p.import.mode.url')}
            </ToggleButton>
          </ToggleButtonGroup>

          {mode === 'yaml' ? (
            <>
              <Stack
                direction="row"
                spacing={1}
                sx={{ alignItems: 'center', flexWrap: 'wrap' }}
              >
                <Button
                  variant="outlined"
                  size="small"
                  onClick={() => void chooseFile()}
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
                    {t('openp2p.import.fileSelected', { name: fileName })}
                  </Typography>
                )}
              </Stack>

              <TextField
                multiline
                fullWidth
                minRows={8}
                maxRows={14}
                label={t('openp2p.import.paste')}
                placeholder={t('openp2p.import.placeholder')}
                value={yaml}
                disabled={applying || previewing}
                onChange={(event) => {
                  setYaml(event.target.value)
                  setFileName('')
                  resetPreview()
                }}
                slotProps={{
                  inputLabel: { shrink: true },
                  htmlInput: { spellCheck: false },
                }}
              />
            </>
          ) : (
            <>
              <TextField
                fullWidth
                size="small"
                type="url"
                label={t('openp2p.import.url')}
                placeholder={t('openp2p.import.urlPlaceholder')}
                value={url}
                disabled={applying || previewing}
                aria-label={t('openp2p.import.url')}
                onChange={(event) => {
                  setUrl(event.target.value)
                  resetPreview()
                }}
                slotProps={{
                  inputLabel: { shrink: true },
                  htmlInput: { spellCheck: false },
                }}
              />
              <Typography variant="caption" color="text.secondary">
                {t('openp2p.import.urlHint')}
              </Typography>
            </>
          )}

          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Button
              variant="contained"
              size="small"
              loading={previewing}
              disabled={applying || previewing}
              onClick={previewCurrent}
            >
              {t('openp2p.import.preview')}
            </Button>
            <Typography
              variant="body2"
              aria-live="polite"
              color={
                preview
                  ? preview.canApply
                    ? 'success.main'
                    : 'error.main'
                  : 'text.secondary'
              }
            >
              {previewing
                ? t('openp2p.import.previewing')
                : preview
                  ? preview.canApply
                    ? t('openp2p.import.canApply')
                    : t('openp2p.import.cannotApply')
                  : ''}
            </Typography>
          </Stack>

          {error && <Alert severity="error">{error}</Alert>}

          {preview && <OpenP2PImportPreviewPanel preview={preview} />}
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
