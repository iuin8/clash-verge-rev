import { HubOutlined } from '@mui/icons-material'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { OpenP2PViewer } from '@/components/openp2p/viewer'
import { getOpenP2pVersion } from '@/services/cmds'
import { useQuery } from '@/services/query-client'

import { SettingItem, SettingList } from '../setting/mods/setting-comp'

export const SettingOpenP2P = () => {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const versionQuery = useQuery({
    queryKey: ['getOpenP2pVersion'],
    queryFn: getOpenP2pVersion,
    staleTime: 60_000,
    retry: 1,
  })
  const version = versionQuery.data?.trim() || null
  return (
    <>
      <SettingList title={t('openp2p.settings.title')}>
        <SettingItem
          onClick={() => setOpen(true)}
          label={t('openp2p.settings.version')}
          secondary={version ? `v${version}` : t('openp2p.settings.unknown')}
          extra={
            <HubOutlined fontSize="small" sx={{ ml: 0.5, opacity: 0.7 }} />
          }
        />
      </SettingList>
      <OpenP2PViewer
        open={open}
        version={version}
        onClose={() => setOpen(false)}
      />
    </>
  )
}
