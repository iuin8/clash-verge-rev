import { Typography } from '@mui/material'
import { useLockFn } from 'ahooks'
import { useCallback } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog } from '@/components/base'
import { useEditorDocument } from '@/hooks/use-editor-document'
import { readProfileFile, saveProfileFile } from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { revalidateQuery } from '@/services/query-client'

import { EditorViewer } from './editor-viewer'
import { EnhanceHint } from './enhance-hint'
import { GroupsEditorViewer } from './groups-editor-viewer'
import { ProxiesEditorViewer } from './proxies-editor-viewer'
import { QrViewer } from './qr-viewer'
import { RulesEditorViewer } from './rules-editor-viewer'

/** Returns the saved text, or false when the backend rejected the write. */
const saveDocument = async (uid: string, value: string) => {
  const result = await mutate(() => saveProfileFile(uid, value), {
    id: `save-profile-file:${uid}`,
    errorNotice: false,
  })
  return result.ok && result.value === true ? value : false
}

interface ProfileItemDialogControl {
  open: boolean
  onClose: () => void
}

export interface ProfileItemDialogState {
  file: ProfileItemDialogControl
  rules: ProfileItemDialogControl
  proxies: ProfileItemDialogControl
  groups: ProfileItemDialogControl
  merge: ProfileItemDialogControl
  script: ProfileItemDialogControl
  confirm: ProfileItemDialogControl
  qr: ProfileItemDialogControl
}

interface Props {
  uid: string
  name: string
  itemData: IProfileItem
  option?: IProfileOption | null
  openP2pSubscription: boolean
  onSave?: (prev?: string, curr?: string) => void
  onDelete: () => void
  mutateProfiles: () => Promise<void>
  dialogs: ProfileItemDialogState
}

export const ProfileItemDialogs = ({
  uid,
  name,
  itemData,
  option,
  openP2pSubscription,
  onSave,
  onDelete,
  mutateProfiles,
  dialogs,
}: Props) => {
  const { t } = useTranslation()

  const loadProfileDocument = useCallback(() => readProfileFile(uid), [uid])
  const loadMergeDocument = useCallback(
    () => readProfileFile(option?.merge ?? ''),
    [option?.merge],
  )
  const loadScriptDocument = useCallback(
    () => readProfileFile(option?.script ?? ''),
    [option?.script],
  )

  const profileDocument = useEditorDocument({
    open: dialogs.file.open,
    load: loadProfileDocument,
  })
  const mergeDocument = useEditorDocument({
    open: dialogs.merge.open,
    load: loadMergeDocument,
  })
  const scriptDocument = useEditorDocument({
    open: dialogs.script.open,
    load: loadScriptDocument,
  })

  const handleSaveProfileDocument = useLockFn(async () => {
    const currentValue = await saveDocument(uid, profileDocument.value)
    // Keep the source draft dirty until the backend confirms the reimport.
    if (currentValue === false) return false
    if (openP2pSubscription) {
      void revalidateQuery(['getOpenP2pState']).catch(() => {})
      void mutateProfiles()
    }
    onSave?.(profileDocument.savedValue, currentValue)
    profileDocument.markSaved(currentValue)
  })

  const handleSaveMergeDocument = useLockFn(async () => {
    const mergeUid = option?.merge ?? ''
    const currentValue = await saveDocument(mergeUid, mergeDocument.value)
    if (currentValue === false) {
      await mergeDocument.reload()
      return
    }
    onSave?.(mergeDocument.savedValue, currentValue)
    mergeDocument.markSaved(currentValue)
  })

  const handleSaveScriptDocument = useLockFn(async () => {
    const scriptUid = option?.script ?? ''
    const currentValue = await saveDocument(scriptUid, scriptDocument.value)
    if (currentValue === false) {
      await scriptDocument.reload()
      return
    }
    onSave?.(scriptDocument.savedValue, currentValue)
    scriptDocument.markSaved(currentValue)
  })

  return (
    <>
      {dialogs.file.open && (
        <EditorViewer
          open={true}
          value={profileDocument.value}
          language="yaml"
          path={`profile:${uid}.yaml`}
          loading={profileDocument.loading}
          dirty={profileDocument.dirty}
          onChange={profileDocument.setValue}
          onSave={handleSaveProfileDocument}
          onClose={dialogs.file.onClose}
        />
      )}
      {dialogs.rules.open && (
        <RulesEditorViewer
          groupsUid={option?.groups ?? ''}
          mergeUid={option?.merge ?? ''}
          profileUid={uid}
          property={option?.rules ?? ''}
          open={true}
          onSave={onSave}
          onClose={dialogs.rules.onClose}
        />
      )}
      {dialogs.proxies.open && (
        <ProxiesEditorViewer
          profileUid={uid}
          property={option?.proxies ?? ''}
          open={true}
          onSave={onSave}
          onClose={dialogs.proxies.onClose}
        />
      )}
      {dialogs.groups.open && (
        <GroupsEditorViewer
          mergeUid={option?.merge ?? ''}
          proxiesUid={option?.proxies ?? ''}
          profileUid={uid}
          property={option?.groups ?? ''}
          open={true}
          onSave={onSave}
          onClose={dialogs.groups.onClose}
        />
      )}
      {dialogs.merge.open && (
        <EditorViewer
          open={true}
          title={t('profiles.components.menu.extendConfig')}
          description={<EnhanceHint stage="profileMerge" />}
          value={mergeDocument.value}
          language="yaml"
          path={`merge:${option?.merge ?? ''}.yaml`}
          loading={mergeDocument.loading}
          dirty={mergeDocument.dirty}
          onChange={mergeDocument.setValue}
          onSave={handleSaveMergeDocument}
          onClose={dialogs.merge.onClose}
        />
      )}
      {dialogs.script.open && (
        <EditorViewer
          open={true}
          title={t('profiles.components.menu.extendScript')}
          description={<EnhanceHint stage="profileScript" />}
          value={scriptDocument.value}
          language="javascript"
          path={`script:${option?.script ?? ''}.js`}
          loading={scriptDocument.loading}
          dirty={scriptDocument.dirty}
          onChange={scriptDocument.setValue}
          onSave={handleSaveScriptDocument}
          onClose={dialogs.script.onClose}
        />
      )}

      <BaseDialog
        title={t('profiles.modals.confirmDelete.title')}
        open={dialogs.confirm.open}
        okBtn={t('shared.actions.confirm')}
        cancelBtn={t('shared.actions.cancel')}
        contentSx={{ width: { xs: 320, sm: 420 }, userSelect: 'text' }}
        onCancel={dialogs.confirm.onClose}
        onClose={dialogs.confirm.onClose}
        onOk={() => {
          onDelete()
          dialogs.confirm.onClose()
        }}
      >
        <Typography variant="body2" sx={{ wordBreak: 'break-word' }}>
          {t('profiles.modals.confirmDelete.message')}
        </Typography>
      </BaseDialog>
      {dialogs.qr.open && itemData.url && (
        <QrViewer
          open={true}
          value={`${itemData.url}${itemData.url.includes('?') ? '&' : '?'}name=${encodeURIComponent(name)}`}
          onClose={dialogs.qr.onClose}
        />
      )}
    </>
  )
}
