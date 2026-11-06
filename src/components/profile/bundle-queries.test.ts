import { expect, it, vi } from 'vitest'

import { revalidateQuery } from '@/services/query-client'

import {
  BUNDLE_CONFLICTS_QUERY_KEY,
  BUNDLE_VIEW_QUERY_KEY,
  refreshBundleQueries,
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
