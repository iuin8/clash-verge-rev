/** UID of the single generated Profile Bundle item (see backend `bundle::BUNDLE_UID`). */
const BUNDLE_UID = 'bundle-main'

/** Stored default name written by the backend; display it translated. */
const DEFAULT_BUNDLE_NAME = 'Profile Bundle'

export const isBundleUid = (uid?: string | null) => uid === BUNDLE_UID

export const isDefaultBundleName = (name?: string | null) =>
  name === DEFAULT_BUNDLE_NAME
