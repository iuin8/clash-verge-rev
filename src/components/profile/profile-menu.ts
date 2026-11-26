export type ProfileMenuAction =
  | 'home'
  | 'select'
  | 'shareQrCode'
  | 'editInfo'
  | 'editFile'
  | 'editRules'
  | 'editProxies'
  | 'editGroups'
  | 'extendConfig'
  | 'extendScript'
  | 'openFile'
  | 'update'
  | 'updateViaProxy'
  | 'delete'

interface ProfileMenuSource {
  url?: string | null
  home?: string | null
}

/**
 * OpenP2P subscriptions are local profiles backed by a subscription store: the
 * raw source is editable and refreshable, but enhancer overlays and the
 * subscription-specific home/QR/update routes do not apply.
 */
export const profileMenuActions = (
  item: ProfileMenuSource,
  managedOpenP2p: boolean,
): ProfileMenuAction[] => {
  const hasUrl = !!item.url
  const actions: ProfileMenuAction[] = []

  if (!managedOpenP2p && item.home) actions.push('home')
  actions.push('select')
  if (!managedOpenP2p && hasUrl) actions.push('shareQrCode')
  actions.push('editInfo', 'editFile')
  if (!managedOpenP2p) {
    actions.push(
      'editRules',
      'editProxies',
      'editGroups',
      'extendConfig',
      'extendScript',
    )
  }
  actions.push('openFile')
  if (hasUrl || managedOpenP2p) actions.push('update')
  if (!managedOpenP2p && hasUrl) actions.push('updateViaProxy')
  actions.push('delete')

  return actions
}
