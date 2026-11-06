/** UID of the single generated Profile Bundle item (see backend `bundle::BUNDLE_UID`). */
export const BUNDLE_UID = 'bundle-main'

export const isBundleUid = (uid?: string | null) => uid === BUNDLE_UID
