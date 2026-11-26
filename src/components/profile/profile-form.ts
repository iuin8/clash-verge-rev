import { isOpenP2pSubscriptionUid } from './openp2p-subscription'

export const isManagedOpenP2pProfile = (uid?: string | null) =>
  isOpenP2pSubscriptionUid(uid)

/**
 * The subscription source owns the identity fields. The shared profile form may
 * still persist every generic fetch/update option the backend honors for an
 * OpenP2P source download.
 */
export const managedOpenP2pPatch = (
  option?: IProfileOption,
): { option: IProfileOption } => ({
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
