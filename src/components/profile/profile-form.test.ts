import { expect, it } from 'vitest'

import { isManagedOpenP2pProfile, managedOpenP2pPatch } from './profile-form'

it('treats only op2p-sub profiles as managed OpenP2P profiles', () => {
  expect(isManagedOpenP2pProfile('op2p-sub-home')).toBe(true)
  expect(isManagedOpenP2pProfile('op2p-sub-')).toBe(false)
  expect(isManagedOpenP2pProfile('local-profile')).toBe(false)
  expect(isManagedOpenP2pProfile(undefined)).toBe(false)
})

it('persists generic fetch/update fields and drops enhancer fields', () => {
  expect(
    managedOpenP2pPatch({
      update_interval: 30,
      allow_auto_update: false,
      with_proxy: true,
      self_proxy: false,
      user_agent: 'agent',
      timeout_seconds: 5,
      danger_accept_invalid_certs: true,
      rules: 'rules',
      merge: 'merge',
      script: 'script',
      proxies: 'proxies',
      groups: 'groups',
    }),
  ).toEqual({
    option: {
      user_agent: 'agent',
      with_proxy: true,
      self_proxy: false,
      update_interval: 30,
      timeout_seconds: 5,
      danger_accept_invalid_certs: true,
      allow_auto_update: false,
    },
  })
})
