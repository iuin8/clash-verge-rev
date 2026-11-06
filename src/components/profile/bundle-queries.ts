import { revalidateQuery } from '@/services/query-client'

export const BUNDLE_VIEW_QUERY_KEY = ['getBundleView'] as const
export const BUNDLE_CONFLICTS_QUERY_KEY = ['getBundleConflicts'] as const

/** Revalidate bundle metadata and conflicts without blocking the caller. */
export const refreshBundleQueries = () =>
  Promise.all([
    revalidateQuery(BUNDLE_VIEW_QUERY_KEY),
    revalidateQuery(BUNDLE_CONFLICTS_QUERY_KEY),
  ])
