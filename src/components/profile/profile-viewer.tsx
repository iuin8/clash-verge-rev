import {
  Box,
  FormControl,
  InputAdornment,
  InputLabel,
  MenuItem,
  Select,
  styled,
  TextField,
} from '@mui/material'
import { useLockFn } from 'ahooks'
import type { Ref } from 'react'
import { useEffect, useImperativeHandle, useRef, useState } from 'react'
import { Controller, useForm } from 'react-hook-form'
import { useTranslation } from 'react-i18next'

import { BaseDialog, Switch } from '@/components/base'
import { useProfiles } from '@/hooks/use-profiles'
import {
  createProfile,
  detectOpenP2pSource,
  patchProfile,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { showNotice } from '@/services/notice-service'
import { version } from '@root/package.json'

import { FileInput } from './file-input'
import { isManagedOpenP2pProfile, managedOpenP2pPatch } from './profile-form'

interface Props {
  onChange: (isActivating?: boolean) => void
  onOpenP2PImport: (seed: { yaml?: string; url?: string }) => void
}

export interface ProfileViewerRef {
  create: () => void
  edit: (item: IProfileItem) => void
}

type ProfileViewerProps = Props & { ref?: Ref<ProfileViewerRef> }

// 同后端 constants::profile::MIN_UPDATE_INTERVAL
const MIN_UPDATE_INTERVAL = 1440

export function ProfileViewer({
  onChange,
  onOpenP2PImport,
  ref,
}: ProfileViewerProps) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [openType, setOpenType] = useState<'new' | 'edit'>('new')
  const [loading, setLoading] = useState(false)
  const { profiles } = useProfiles()

  const fileDataRef = useRef<string | null>(null)

  const { control, watch, setValue, reset, handleSubmit, getValues } =
    useForm<IProfileItem>({
      defaultValues: {
        type: 'remote',
        name: '',
        desc: '',
        url: '',
        option: {
          with_proxy: false,
          self_proxy: false,
          allow_auto_update: true,
        },
      },
    })

  useImperativeHandle(ref, () => ({
    create: () => {
      setOpenType('new')
      setOpen(true)
    },
    edit: (item: IProfileItem) => {
      if (item) {
        reset(item)
      }
      setOpenType('edit')
      setOpen(true)
    },
  }))

  const selfProxy = watch('option.self_proxy')
  const withProxy = watch('option.with_proxy')

  useEffect(() => {
    if (selfProxy) setValue('option.with_proxy', false)
  }, [selfProxy, setValue])

  useEffect(() => {
    if (withProxy) setValue('option.self_proxy', false)
  }, [setValue, withProxy])

  const handleOk = useLockFn(
    handleSubmit(async (form) => {
      setLoading(true)
      try {
        const uid = form.uid
        const managedOpenP2p =
          openType === 'edit' && isManagedOpenP2pProfile(uid)
        if (!managedOpenP2p) {
          if (!form.type) {
            throw new Error(
              t('profiles.modals.profileForm.errors.typeRequired'),
            )
          }
          if (form.type === 'remote' && !form.url) {
            throw new Error(t('profiles.modals.profileForm.errors.urlRequired'))
          }
        }

        const option = form.option ? { ...form.option } : undefined
        if (option?.timeout_seconds) {
          option.timeout_seconds = +option.timeout_seconds
        } else if (option) {
          option.timeout_seconds = undefined
        }
        if (option?.update_interval) {
          option.update_interval = +option.update_interval
        } else if (option) {
          option.update_interval = undefined
        }
        if (option?.user_agent === '') {
          option.user_agent = undefined
        }

        if (managedOpenP2p) {
          if (!uid) {
            throw new Error(t('profiles.modals.profileForm.errors.uidMissing'))
          }
          await mutate(
            () =>
              patchProfile(
                uid,
                managedOpenP2pPatch({
                  name: form.name,
                  desc: form.desc,
                  option,
                }),
              ),
            {
              id: 'patch-profile',
              errorNotice: false,
            },
          )
          setOpen(false)
          setTimeout(() => reset(), 500)
          setTimeout(() => onChange(false), 0)
          return
        }

        const name = form.name || `${form.type} file`
        const item = {
          type: form.type,
          name,
          desc: form.desc,
          option,
          ...(form.type === 'remote' ? { url: form.url } : {}),
        }
        const isRemote = form.type === 'remote'
        const isUpdate = openType === 'edit'

        const isActivating = isUpdate && form.uid === (profiles?.current ?? '')

        if (openType === 'new') {
          const remoteUrl = form.url ?? ''
          const detected = await detectOpenP2pSource(
            isRemote ? { url: remoteUrl } : { yaml: fileDataRef.current ?? '' },
          ).catch(() => false)
          if (detected) {
            const seed = isRemote
              ? { url: remoteUrl }
              : { yaml: fileDataRef.current ?? '' }
            setOpen(false)
            fileDataRef.current = null
            setTimeout(() => reset(), 500)
            onOpenP2PImport(seed)
            return
          }
        }

        // Preserve proxy settings when the remote retry succeeds through another route.
        const originalOptions = {
          with_proxy: form.option?.with_proxy,
          self_proxy: form.option?.self_proxy,
        }

        if (!isRemote) {
          if (openType === 'new') {
            await mutate(() => createProfile(item, fileDataRef.current), {
              id: 'create-profile',
              errorNotice: false,
            })
          } else {
            if (!form.uid) {
              throw new Error(
                t('profiles.modals.profileForm.errors.uidMissing'),
              )
            }
            await mutate(() => patchProfile(form.uid!, item), {
              id: 'patch-profile',
              errorNotice: false,
            })
          }
        } else {
          try {
            if (openType === 'new') {
              await mutate(() => createProfile(item, fileDataRef.current), {
                id: 'create-profile',
                errorNotice: false,
              })
            } else {
              if (!form.uid) {
                throw new Error(
                  t('profiles.modals.profileForm.errors.uidMissing'),
                )
              }
              await mutate(() => patchProfile(form.uid!, item), {
                id: 'patch-profile',
                errorNotice: false,
              })
            }
          } catch {
            showNotice.info(
              'profiles.modals.profileForm.feedback.notifications.creationRetry',
            )

            const retryItem = {
              ...item,
              option: {
                ...item.option,
                with_proxy: false,
                self_proxy: true,
              },
            }

            if (openType === 'new') {
              await mutate(
                () => createProfile(retryItem, fileDataRef.current),
                {
                  id: 'create-profile',
                  errorNotice: false,
                },
              )
            } else {
              if (!form.uid) {
                throw new Error(
                  t('profiles.modals.profileForm.errors.uidMissing'),
                )
              }
              await mutate(() => patchProfile(form.uid!, retryItem), {
                id: 'patch-profile',
                errorNotice: false,
              })

              await mutate(
                () => patchProfile(form.uid!, { option: originalOptions }),
                {
                  id: 'patch-profile',
                  errorNotice: false,
                },
              )
            }

            showNotice.success(
              'profiles.modals.profileForm.feedback.notifications.creationSuccess',
            )
          }
        }

        setOpen(false)
        setTimeout(() => reset(), 500)
        fileDataRef.current = null

        setTimeout(() => {
          onChange(isActivating)
        }, 0)
      } catch (err) {
        showNotice.error('profiles.modals.profileForm.errors.saveFailed', err)
      } finally {
        setLoading(false)
      }
    }),
  )

  const handleClose = () => {
    try {
      setOpen(false)
      fileDataRef.current = null
      setTimeout(() => reset(), 500)
    } catch (e) {
      console.warn('[ProfileViewer] handleClose error:', e)
    }
  }

  const text = {
    fullWidth: true,
    size: 'small',
    margin: 'normal',
    variant: 'outlined',
    autoComplete: 'off',
    autoCorrect: 'off',
  } as const

  const formType = watch('type')
  const isRemote = formType === 'remote'
  const isLocal = formType === 'local'
  const managedOpenP2p =
    openType === 'edit' && isManagedOpenP2pProfile(watch('uid'))

  const updateIntervalField = (
    <Controller
      name="option.update_interval"
      control={control}
      render={({ field }) => {
        const interval = Number(field.value)
        const tooFrequent =
          Number.isFinite(interval) &&
          interval > 0 &&
          interval < MIN_UPDATE_INTERVAL

        return (
          <TextField
            {...text}
            {...field}
            type="number"
            label={t('profiles.modals.profileForm.fields.updateInterval')}
            helperText={
              tooFrequent
                ? t('profiles.modals.profileForm.warnings.frequentUpdate', {
                    minutes: MIN_UPDATE_INTERVAL,
                  })
                : undefined
            }
            slotProps={{
              formHelperText: { sx: { color: 'warning.main' } },
              input: {
                endAdornment: (
                  <InputAdornment position="end">
                    {t('shared.units.minutes')}
                  </InputAdornment>
                ),
              },
            }}
          />
        )
      }}
    />
  )

  const allowAutoUpdateField = (
    <Controller
      name="option.allow_auto_update"
      control={control}
      render={({ field }) => (
        <StyledBox>
          <InputLabel>
            {t('profiles.modals.profileForm.fields.allowAutoUpdate')}
          </InputLabel>
          <Switch checked={field.value ?? true} {...field} color="primary" />
        </StyledBox>
      )}
    />
  )

  return (
    <BaseDialog
      open={open}
      title={
        openType === 'new'
          ? t('profiles.modals.profileForm.title.create')
          : t('profiles.modals.profileForm.title.edit')
      }
      contentSx={{ width: 375, pb: 0, maxHeight: '80%' }}
      okBtn={t('shared.actions.save')}
      cancelBtn={t('shared.actions.cancel')}
      onClose={handleClose}
      onCancel={handleClose}
      onOk={handleOk}
      loading={loading}
    >
      {managedOpenP2p ? (
        <TextField
          {...text}
          sx={{ mt: 1, mb: 1 }}
          label={t('profiles.modals.profileForm.fields.type')}
          value={t('profiles.components.profileItem.openp2p')}
          slotProps={{ input: { readOnly: true } }}
        />
      ) : (
        <Controller
          name="type"
          control={control}
          render={({ field }) => (
            <FormControl size="small" fullWidth sx={{ mt: 1, mb: 1 }}>
              <InputLabel>
                {t('profiles.modals.profileForm.fields.type')}
              </InputLabel>
              <Select
                {...field}
                autoFocus
                label={t('profiles.modals.profileForm.fields.type')}
              >
                <MenuItem value="remote">
                  {t('profiles.modals.profileForm.types.remote')}
                </MenuItem>
                <MenuItem value="local">
                  {t('profiles.modals.profileForm.types.local')}
                </MenuItem>
              </Select>
            </FormControl>
          )}
        />
      )}

      <Controller
        name="name"
        control={control}
        render={({ field }) => (
          <TextField {...text} {...field} label={t('shared.labels.name')} />
        )}
      />

      <Controller
        name="desc"
        control={control}
        render={({ field }) => (
          <TextField
            {...text}
            {...field}
            label={t('profiles.modals.profileForm.fields.description')}
          />
        )}
      />

      {isLocal && openType === 'new' && (
        <FileInput
          onChange={(file, val) => {
            setValue('name', getValues('name') || file.name)
            fileDataRef.current = val
          }}
        />
      )}

      {(isRemote || managedOpenP2p) && (
        <>
          {isRemote && !managedOpenP2p && (
            <Controller
              name="url"
              control={control}
              render={({ field }) => (
                <TextField
                  {...text}
                  {...field}
                  multiline
                  label={t(
                    'profiles.modals.profileForm.fields.subscriptionUrl',
                  )}
                />
              )}
            />
          )}

          <Controller
            name="option.user_agent"
            control={control}
            render={({ field }) => (
              <TextField
                {...text}
                {...field}
                placeholder={`clash-verge/v${version}`}
                label={t('profiles.modals.profileForm.fields.userAgent')}
              />
            )}
          />

          <Controller
            name="option.timeout_seconds"
            control={control}
            render={({ field }) => (
              <TextField
                {...text}
                {...field}
                type="number"
                placeholder="60"
                label={t('profiles.modals.profileForm.fields.httpTimeout')}
                slotProps={{
                  input: {
                    endAdornment: (
                      <InputAdornment position="end">
                        {t('shared.units.seconds')}
                      </InputAdornment>
                    ),
                  },
                }}
              />
            )}
          />
          {updateIntervalField}
          <Controller
            name="option.with_proxy"
            control={control}
            render={({ field }) => (
              <StyledBox>
                <InputLabel>
                  {t('profiles.modals.profileForm.fields.useSystemProxy')}
                </InputLabel>
                <Switch checked={field.value!} {...field} color="primary" />
              </StyledBox>
            )}
          />

          <Controller
            name="option.self_proxy"
            control={control}
            render={({ field }) => (
              <StyledBox>
                <InputLabel>
                  {t('profiles.modals.profileForm.fields.useClashProxy')}
                </InputLabel>
                <Switch checked={field.value!} {...field} color="primary" />
              </StyledBox>
            )}
          />

          <Controller
            name="option.danger_accept_invalid_certs"
            control={control}
            render={({ field }) => (
              <StyledBox>
                <InputLabel>
                  {t('profiles.modals.profileForm.fields.acceptInvalidCerts')}
                </InputLabel>
                <Switch checked={field.value!} {...field} color="primary" />
              </StyledBox>
            )}
          />

          {allowAutoUpdateField}
        </>
      )}
    </BaseDialog>
  )
}

const StyledBox = styled(Box)(() => ({
  margin: '8px 0 8px 8px',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
}))
