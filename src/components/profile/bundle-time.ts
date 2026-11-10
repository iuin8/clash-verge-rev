import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'

dayjs.extend(relativeTime)

export interface BundleRefreshTime {
  /** Localized relative label (e.g. "2 minutes ago"); null when unknown. */
  relative: string | null
  /** Full timestamp for accessible titles; null when unknown. */
  absolute: string | null
}

/** Format `BundleView.generated_at` (Unix seconds); never renders the 1970 epoch. */
export const bundleRefreshTime = (
  unixSeconds?: number | null,
): BundleRefreshTime => {
  if (!unixSeconds || unixSeconds <= 0) {
    return { relative: null, absolute: null }
  }
  const date = dayjs(unixSeconds * 1000)
  return {
    relative: date.fromNow(),
    absolute: date.format('YYYY-MM-DD HH:mm'),
  }
}
