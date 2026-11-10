import { expect, it } from 'vitest'

import { bundleRefreshTime } from './bundle-time'

it('returns a neutral placeholder for unknown generation times', () => {
  expect(bundleRefreshTime(0)).toEqual({ relative: null, absolute: null })
  expect(bundleRefreshTime(undefined)).toEqual({
    relative: null,
    absolute: null,
  })
})

it('formats a generated timestamp as relative and absolute labels', () => {
  const result = bundleRefreshTime(1_700_000_000)

  expect(result.relative).toBeTruthy()
  expect(result.absolute).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/)
})
