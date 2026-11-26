import { isOpenP2pSubscriptionUid } from './openp2p-subscription'

export const isManagedOpenP2pProfile = (uid?: string | null) =>
  isOpenP2pSubscriptionUid(uid)

interface ManagedOpenP2pEdit {
  name?: string | null
  desc?: string | null
  option?: IProfileOption
}

/**
 * The shared edit-info form writes the display identity and every generic
 * fetch/update option the backend honors for an OpenP2P source download.
 * Empty identity fields clear the stored override.
 */
export const managedOpenP2pPatch = ({
  name,
  desc,
  option,
}: ManagedOpenP2pEdit) => ({
  name: name ?? '',
  desc: desc ?? '',
  option: {
    user_agent: option?.user_agent,
    with_proxy: option?.with_proxy,
    self_proxy: option?.self_proxy,
    update_interval: option?.update_interval,
    timeout_seconds: option?.timeout_seconds,
    danger_accept_invalid_certs: option?.danger_accept_invalid_certs,
    allow_auto_update: option?.allow_auto_update,
  },
})
