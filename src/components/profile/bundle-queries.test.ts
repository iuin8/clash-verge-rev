import { expect, it, vi } from 'vitest'

import { revalidateQuery } from '@/services/query-client'

import {
  BUNDLE_CONFLICTS_QUERY_KEY,
  BUNDLE_VIEW_QUERY_KEY,
  refreshBundleQueries,
  shouldRetryBundleRefresh,
} from './bundle-queries'

vi.mock('@/services/query-client', () => ({
  revalidateQuery: vi.fn(() => Promise.resolve(undefined)),
}))

it('revalidates bundle view and conflicts', async () => {
  await refreshBundleQueries()

  expect(revalidateQuery).toHaveBeenCalledTimes(2)
  expect(revalidateQuery).toHaveBeenNthCalledWith(1, BUNDLE_VIEW_QUERY_KEY)
  expect(revalidateQuery).toHaveBeenNthCalledWith(2, BUNDLE_CONFLICTS_QUERY_KEY)
})

it('retries a slow-finalize refresh only while still stale and not cancelled', () => {
  expect(shouldRetryBundleRefresh(false, true)).toBe(true)
  expect(shouldRetryBundleRefresh(false, false)).toBe(false)
  expect(shouldRetryBundleRefresh(false, undefined)).toBe(false)
  expect(shouldRetryBundleRefresh(true, true)).toBe(false)
})
