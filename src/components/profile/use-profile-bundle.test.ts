import { expect, it, vi } from 'vitest'

import {
  bundleSelectionState,
  selectableBundleProfiles,
  selectAllBundleProfiles,
  selectedBundleMembers,
  toggleBundleProfileSelection,
} from './use-profile-bundle'

vi.mock('@/services/cmds', () => ({
  deleteBundle: vi.fn(),
  getBundleConflicts: vi.fn(),
  getBundleView: vi.fn(),
  refreshBundle: vi.fn(),
  setBundleMembers: vi.fn(),
}))
vi.mock('@/services/mutate', () => ({ mutate: vi.fn() }))
vi.mock('@/services/notice-service', () => ({
  showNotice: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  },
}))
vi.mock('@/services/query-client', () => ({
  revalidateQuery: vi.fn(() => Promise.resolve()),
  useQuery: vi.fn(),
}))

const profile = (uid: string, name?: string): IProfileItem => ({
  uid,
  type: 'local',
  name: name ?? uid,
  file: null,
  updated: null,
})

const items: IProfileItem[] = [
  profile('normal-a'),
  profile('op2p-sub-alpha'),
  profile('normal-b'),
  profile('op2p-sub-beta'),
  profile('bundle-main'),
]

it('offers subscription cards as selectable members but not the bundle', () => {
  expect(selectableBundleProfiles(items).map((item) => item.uid)).toEqual([
    'normal-a',
    'op2p-sub-alpha',
    'normal-b',
    'op2p-sub-beta',
  ])
})

it('select-all picks every ordinary profile and at most one subscription', () => {
  expect([...selectAllBundleProfiles(items, new Set())].sort()).toEqual([
    'normal-a',
    'normal-b',
    'op2p-sub-alpha',
  ])
})

it('select-all keeps the subscription the user already picked', () => {
  const selected = selectAllBundleProfiles(
    items,
    new Set(['normal-a', 'op2p-sub-beta']),
  )
  expect(selected.has('op2p-sub-beta')).toBe(true)
  expect(selected.has('op2p-sub-alpha')).toBe(false)
})

it('refuses a second subscription and keeps the first one selected', () => {
  const first = toggleBundleProfileSelection(
    new Set(['normal-a', 'op2p-sub-alpha']),
    'op2p-sub-beta',
  )
  expect(first.refused).toBe(true)
  expect(first.selected.has('op2p-sub-beta')).toBe(false)
  expect(first.selected.has('op2p-sub-alpha')).toBe(true)
})

it('lets the selected subscription be deselected and replaced', () => {
  const deselected = toggleBundleProfileSelection(
    new Set(['op2p-sub-alpha']),
    'op2p-sub-alpha',
  )
  expect(deselected.refused).toBe(false)
  expect(deselected.selected.size).toBe(0)

  const replaced = toggleBundleProfileSelection(
    deselected.selected,
    'op2p-sub-beta',
  )
  expect(replaced.refused).toBe(false)
  expect(replaced.selected.has('op2p-sub-beta')).toBe(true)
})

it('reports all only when every ordinary profile and one subscription are selected', () => {
  const selectable = selectableBundleProfiles(items)
  expect(bundleSelectionState(selectable, new Set())).toBe('none')
  expect(
    bundleSelectionState(selectable, new Set(['normal-a', 'op2p-sub-alpha'])),
  ).toBe('partial')
  expect(
    bundleSelectionState(
      selectable,
      new Set(['normal-a', 'normal-b', 'op2p-sub-alpha']),
    ),
  ).toBe('all')
  expect(
    bundleSelectionState(
      selectable,
      new Set(['normal-a', 'normal-b', 'op2p-sub-alpha', 'op2p-sub-beta']),
    ),
  ).toBe('partial')
})

it('builds a draft with at most one subscription even from a stale selection', () => {
  const members = selectedBundleMembers(
    items,
    new Set([
      'normal-a',
      'normal-b',
      'op2p-sub-alpha',
      'op2p-sub-beta',
      'bundle-main',
    ]),
  )
  expect(members).toEqual([
    { uid: 'normal-a', name: 'normal-a' },
    { uid: 'op2p-sub-alpha', name: 'op2p-sub-alpha' },
    { uid: 'normal-b', name: 'normal-b' },
  ])
})
