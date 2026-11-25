const OPENP2P_SUBSCRIPTION_UID_PREFIX = 'op2p-sub-'

/** Canonical profile uid, independent of a cached profiles.yaml item. */
export const isOpenP2pSubscriptionUid = (uid: string | null | undefined) =>
  typeof uid === 'string' &&
  uid.startsWith(OPENP2P_SUBSCRIPTION_UID_PREFIX) &&
  uid.length > OPENP2P_SUBSCRIPTION_UID_PREFIX.length
