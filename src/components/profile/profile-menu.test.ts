import { expect, it } from 'vitest'

import { profileMenuActions } from './profile-menu'

it('keeps an OpenP2P source on generic capabilities only', () => {
  expect(profileMenuActions({ url: null, home: null }, true)).toEqual([
    'select',
    'editInfo',
    'editFile',
    'openFile',
    'update',
    'delete',
  ])
})

it('keeps remote subscription actions and enhancer overlays', () => {
  expect(
    profileMenuActions(
      { url: 'https://example.com/profile.yaml', home: 'https://example.com' },
      false,
    ),
  ).toEqual([
    'home',
    'select',
    'shareQrCode',
    'editInfo',
    'editFile',
    'editRules',
    'editProxies',
    'editGroups',
    'extendConfig',
    'extendScript',
    'openFile',
    'update',
    'updateViaProxy',
    'delete',
  ])
})

it('does not offer update actions for a plain local file', () => {
  const actions = profileMenuActions({ url: null, home: null }, false)
  expect(actions).not.toContain('update')
  expect(actions).not.toContain('updateViaProxy')
  expect(actions).not.toContain('shareQrCode')
  expect(actions).not.toContain('home')
})
